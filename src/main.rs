use anyhow::{anyhow, Context, Result};
use embedded_svc::http::Method;
use embedded_svc::ipv4::Ipv4Addr;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::server::{Configuration as ServerConfig, EspHttpServer};
use esp_idf_svc::io::Write;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{
    AccessPointConfiguration as ApConfiguration, AuthMethod, ClientConfiguration,
    Configuration as WifiConfiguration, EspWifi,
};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};
use std::{thread};

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
            b'+' => { out.push(b' '); i += 1; }
            b'%' if i + 2 < bytes.len() => {
                let hv = |c| match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                };
                if let (Some(h), Some(l)) = (hv(bytes[i+1]), hv(bytes[i+2])) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(bytes[i]); i += 1;
                }
            }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn wait_ip(wifi: &EspWifi, timeout: Duration) -> Result<()> {
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

fn connect_sta(wifi: &mut EspWifi, ssid: &str, pass: &str) -> Result<()> {
    let _ = wifi.stop();
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid.try_into().map_err(|_| anyhow!("SSID invalide"))?,
        password: pass.try_into().map_err(|_| anyhow!("MDP invalide"))?,
        ..Default::default()
    }))?;
    wifi.start()?;
    wifi.connect()?;
    wait_ip(wifi, Duration::from_secs(20))
}

fn start_ap(wifi: &mut EspWifi, ssid: &str) -> Result<()> {
    wifi.set_configuration(&WifiConfiguration::AccessPoint(ApConfiguration {
        ssid: ssid.try_into().unwrap(),
        channel: 6,
        auth_method: AuthMethod::None, // set WPA2 if you want a password
        max_connections: 4,
        ..Default::default()
    }))?;
    wifi.start()?;
    Ok(())
}


fn spawn_server(tx: Sender<SetupReq>) -> Result<EspHttpServer<'static>> {
    let mut server = EspHttpServer::new(&ServerConfig::default())?;

    server.fn_handler("/", Method::Get, |req| -> anyhow::Result<()> {
        let mut r = req.into_ok_response()?;
        r.write_all(
            br#"<!doctype html>
<html><body>
  <h3>ESP32 Setup</h3>
  <form method="post" action="/setup">
    <label>SSID <input name="ssid"></label><br>
    <label>Password <input name="pass" type="password"></label><br>
    <button type="submit">Save & Connect</button>
  </form>
</body></html>"#,
        )?;
        Ok(())
    })?;

    server.fn_handler("/setup", Method::Post, move |mut req| -> anyhow::Result<()> {
        let mut body = Vec::new();
        let mut buf = [0u8; 512];
        loop {
            let n = req.read(&mut buf)?;
            if n == 0 { break; }
            body.extend_from_slice(&buf[..n]);
        }
        let s = core::str::from_utf8(&body).unwrap_or("");
        println!("📡 Received /setup request: raw body = {}", s);

        let mut ssid = String::new();
        let mut pass = String::new();

        for pair in s.split('&') {
            let mut it = pair.splitn(2, '=');
            let key = it.next().unwrap_or("");
            let val = it.next().unwrap_or("");
            let val_decoded = url_decode(val.as_bytes());
            match key {
                "ssid" => ssid = val_decoded,
                "pass" => pass = val_decoded,
                _ => {}
            }
        }

        if ssid.is_empty() {
            let mut r = req.into_response(400, None, &[("Content-Type","text/plain")])?;
            r.write_all(b"Missing ssid")?;
            return Ok(());
        }

        println!("Parsed setup -> ssid='{}', pass_len={}", ssid, pass.len());
        let _ = tx.send(SetupReq { ssid, pass });
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

    start_ap(&mut wifi, "ESP32_SETUP")?;
    println!("AP 'ESP32_SETUP' started. Connect and open http://192.168.71.1/");
    let (tx, rx) = channel::<SetupReq>();
    let _server = spawn_server(tx)?; 
    loop {
        if let Ok(req) = rx.recv() {
            println!("📥 Setup received: ssid='{}'", req.ssid);
            match connect_sta(&mut wifi, &req.ssid, &req.pass) {
                Ok(_) => {
                    println!("Connected to '{}'.", req.ssid);
                }
                Err(e) => {
                    eprintln!("STA connect failed: {e}. Re-enabling AP for retry.");
                    let _ = wifi.stop();
                    if let Err(e2) = start_ap(&mut wifi, "ESP32_SETUP") {
                        eprintln!("Failed to restart AP: {e2}");
                    }
                }
            }
        }
    }
}
