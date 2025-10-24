use anyhow::{anyhow, Context, Result};
use embedded_svc::http::Method;
use embedded_svc::ipv4::Ipv4Addr;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::http::client::{Configuration as HttpCfg, EspHttpConnection};
use esp_idf_svc::io::Write;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::wifi::{
    AuthMethod, ClientConfiguration, Configuration as WifiConfiguration, EspWifi,
};
use std::time::{Duration, Instant};
use std::thread;

use esp_idf_hal::{
    delay::Ets,
    gpio::{PinDriver, Pull},
};
use dht_sensor::{dht11, DhtReading};

// ================= WIFI UTILS =====================

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

fn connect_sta(wifi: &mut EspWifi, ssid: &str, pass: &str) -> Result<()> {
    let _ = wifi.stop();

    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid: ssid.try_into().map_err(|_| anyhow!("SSID invalide"))?,
        password: pass.try_into().map_err(|_| anyhow!("MDP invalide"))?,
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    }))?;

    wifi.start()?;
    wifi.connect()?;

    wait_for_ip(wifi, Duration::from_secs(20))
}

// =============== MAIN ===================

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    unsafe {
        esp_idf_sys::uart_set_baudrate(esp_idf_sys::uart_port_t_UART_NUM_0, 9600);
    }
    EspLogger::initialize_default();

    let peripherals = Peripherals::take().context("No peripherals")?;
    let sysloop = EspSystemEventLoop::take().context("No sysloop")?;
    let mut wifi = EspWifi::new(peripherals.modem, sysloop, None).context("Wi-Fi init")?;

    // 🔐 Hardcoded credentials
    let ssid = "TP-Link_3889";
    let pass = "25209228";

    println!("📡 Connecting to Wi-Fi '{}'...", ssid);
    connect_sta(&mut wifi, ssid, pass)?;
    println!("✅ Connected! Starting DHT11 read loop...");

    // DHT11 sensor on GPIO4
    let mut pin = PinDriver::input_output_od(peripherals.pins.gpio4)?;
    pin.set_pull(Pull::Up)?;
    let mut delay = Ets;

    let url = "http://b15ca8fb2839.ngrok-free.app/ping";

    loop {
        let mut temperature_value: i8 = 0;
        let mut humidity_value: u8 = 0;

        match dht11::Reading::read(&mut delay, &mut pin) {
            Ok(dht11::Reading {
                temperature,
                relative_humidity,
            }) => {
                log::info!("🌡 Temp: {} °C, 💧 Humidity: {} %", temperature, relative_humidity);
                temperature_value = temperature;
                humidity_value = relative_humidity;
            }
            Err(e) => {
                log::warn!("⚠️ DHT11 read error: {:?}", e);
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
        println!("📨 POST Status: {}", resp.status());

        thread::sleep(Duration::from_secs(10));
    }
}
