use anyhow::{anyhow, Context, Result};
use embedded_svc::http::Method;
use embedded_svc::ipv4::Ipv4Addr;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::server::{Configuration as ServerConfig, EspHttpServer};
use esp_idf_svc::http::client::{Configuration as HttpCfg, EspHttpConnection};
use esp_idf_svc::io::Write;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{
    AccessPointConfiguration as ApConfiguration, AuthMethod, ClientConfiguration,
    Configuration as WifiConfiguration, EspWifi,
};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};
use std::{thread};

use esp_idf_hal::{
    delay::Ets,
    gpio::{PinDriver, Pull}
};
use dht_sensor::{dht11, DhtReading};


#[derive(Clone)]
struct SetupReq {
    ssid: String,
    pass: String,
}


fn url_decode(bytes: &[u8]) -> String {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hv = |c| match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                };
                if let (Some(h), Some(l)) = (hv(bytes[i + 1]), hv(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    i += 2;
                } else {
                    out.push(bytes[i]);
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}


fn wait_for_ip(wifi: &EspWifi, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let info = wifi.sta_netif().get_ip_info()?;
        if info.ip != Ipv4Addr::new(0, 0, 0, 0) {
            println!("✅ Got IP: {:?}", info);
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(anyhow!("Timeout DHCP"));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn start_ap(wifi: &mut EspWifi, ssid: &str) -> Result<()> {
    wifi.set_configuration(&WifiConfiguration::AccessPoint(ApConfiguration {
        ssid: ssid.try_into().unwrap(),
        channel: 6,
        auth_method: AuthMethod::None,
        max_connections: 4,
        ..Default::default()
    }))?;
    wifi.start()?;
    println!("📡 AP '{ssid}' started → http://192.168.71.1/");
    Ok(())
}

fn connect_sta(wifi: &mut EspWifi, ssid: &str, pass: &str) -> Result<()> {
    let _ = wifi.stop();
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid.try_into().map_err(|_| anyhow!("SSID invalide"))?,
        password: pass.try_into().map_err(|_| anyhow!("MDP invalide"))?,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wait_for_ip(wifi, Duration::from_secs(20))
}


fn spawn_setup_server(tx: Sender<SetupReq>) -> Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&ServerConfig::default())?;

    server.fn_handler("/", Method::Get, |req| -> anyhow::Result<()> {
        let mut r = req.into_ok_response()?;
        r.write_all(br#"<!doctype html><html><body>
<h3>ESP32 Setup</h3>
<input id=ssid placeholder=SSID>
<input id=pass placeholder=Password type=password>
<button onclick="send()">Connect</button>
<p id=s></p>
<script>
async function send(){
 const ssid=document.getElementById('ssid').value.trim();
 const pass=document.getElementById('pass').value.trim();
 if(!ssid){s.textContent='Missing SSID';return;}
 const body=`ssid=${encodeURIComponent(ssid)}&pass=${encodeURIComponent(pass)}`;
 const r=await fetch('/setup',{method:'POST',headers:{'Content-Type':'application/x-www-form-urlencoded'},body});
 s.textContent=await r.text();
}
</script></body></html>"#)?;
        Ok(())
    })?;

    let tx2 = tx.clone();
    server.fn_handler("/setup", Method::Post, move |mut req| -> anyhow::Result<()> {
        let mut body = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = req.read(&mut buf)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&buf[..n]);
        }

        let body_str = String::from_utf8_lossy(&body);
        let mut ssid = String::new();
        let mut pass = String::new();

        for pair in body_str.split('&') {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("");
            let val = kv.next().unwrap_or("");
            let val_decoded = url_decode(val.as_bytes());
            match key {
                "ssid" => ssid = val_decoded,
                "pass" => pass = val_decoded,
                _ => {}
            }
        }

        println!("📡 Received setup: ssid='{ssid}', pass_len={}", pass.len());
        let _ = tx2.send(SetupReq { ssid, pass });

        let mut r = req.into_ok_response()?;
        r.write_all(b"Accepted. Trying to connect...")?;
        Ok(())
    })?;

    Ok(server)
}

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().context("No peripherals")?;
    let sysloop = EspSystemEventLoop::take().context("No sysloop")?;
    let mut wifi = EspWifi::new(peripherals.modem, sysloop, None).context("Wi-Fi init")?;

    // Mode AP + serveur de setup
    start_ap(&mut wifi, "ESP32_SETUP")?;
    let (tx, rx) = channel::<SetupReq>();
    let server = spawn_setup_server(tx)?;
    println!("🖥️ Waiting for Wi-Fi credentials...");

    let creds = rx.recv().expect("Channel closed");
    drop(server);

    println!("📡 Connecting to '{}'", creds.ssid);
    connect_sta(&mut wifi, &creds.ssid, &creds.pass)?;

    println!("✅ Connected! Starting ADC read loop...");


    let mut pin = PinDriver::input_output_od(peripherals.pins.gpio4)?;
    pin.set_pull(Pull::Up)?;

    let mut delay = Ets;

    let url = "http://b15ca8fb2839.ngrok-free.app/ping";
    loop {
        let mut temperature_value: i8 = 0;
        let mut humidity_value: u8 = 0;

        match dht11::Reading::read(&mut delay, &mut pin) {
            Ok(dht11::Reading { temperature, relative_humidity }) => {
                log::info!("Temp: {} °C, Humidity: {} %", temperature, relative_humidity);
                temperature_value = temperature;
                humidity_value =relative_humidity;
            }
            Err(e) => {
                log::warn!("Read error: {:?}", e);
            }
        }

        // HTTP POST
        let conn = EspHttpConnection::new(&HttpCfg::default())?;
        let mut client = embedded_svc::http::client::Client::wrap(conn);

        let payload = format!(
            r#"{{"ping":true,"temperature":{},"humidity":{}}}"#,
            temperature_value, humidity_value
        );

        let mut req = client.request(
            Method::Post,
            url,
            &[("Content-Type", "application/json")],
        )?;

        req.write_all(payload.as_bytes())?;

        let resp = req.submit()?;
        println!("📨 Status: {}", resp.status());

        thread::sleep(Duration::from_secs(10));
    }
}

