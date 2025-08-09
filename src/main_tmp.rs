use anyhow::{anyhow, Context, Result};
use embedded_svc::http::{Method};
use embedded_svc::ipv4::Ipv4Addr;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use esp_idf_svc::io::Write;
use esp_idf_svc::log::EspLogger;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{ClientConfiguration, Configuration as WifiConfiguration, EspWifi};
use esp_idf_svc::hal::peripherals::Peripherals;
use std::thread;
use std::time::{Duration, Instant};

fn wait_for_ip(wifi: &EspWifi, timeout: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let info = wifi.sta_netif().get_ip_info()?;
        if info.ip != Ipv4Addr::new(0, 0, 0, 0) {
            println!("✅ Got IP: {:?}", info);
            return Ok(());
        }
        if start.elapsed() > timeout {
            return Err(anyhow!("Timeout waiting for DHCP IP"));
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn ensure_wifi_connected(wifi: &mut EspWifi) -> Result<()> {
    if wifi.is_connected().unwrap_or(false) {
        return Ok(());
    }
    println!("📶 Reconnect Wi-Fi…");
    wifi.connect().context("Reconnect Wi-Fi")?;
    wait_for_ip(wifi, Duration::from_secs(20))
}

fn main() -> Result<()> {
    esp_idf_sys::link_patches();
    EspLogger::initialize_default();
    println!("📡 Init…");

    // Peripherals / eventloop / NVS
    let peripherals = Peripherals::take().context("Pas de périphériques")?;
    let sysloop = EspSystemEventLoop::take().context("Pas de sysloop")?;
    let nvs = EspDefaultNvsPartition::take().context("Pas de NVS")?;

    // ⚠️ SSID 2.4 GHz uniquement sur ESP32-C3
    let ssid = "SFR_7500".try_into().map_err(|_| anyhow!("SSID invalide"))?;
    let pass = "axqj5smh95nhyk7bfn23".try_into().map_err(|_| anyhow!("MDP invalide"))?;

    let mut wifi = EspWifi::new(peripherals.modem, sysloop, Some(nvs)).context("Création Wi-Fi")?;
    wifi.set_configuration(&WifiConfiguration::Client(ClientConfiguration {
        ssid,
        password: pass,
        ..Default::default()
    }))?;

    println!("🚀 Démarrage Wi-Fi…");
    wifi.start().context("Start Wi-Fi")?;
    wifi.connect().context("Connect Wi-Fi")?;
    wait_for_ip(&wifi, Duration::from_secs(20))?;

    let url = "http://ba188219f2d8.ngrok-free.app/ping";

    // Boucle principale : envoie un POST toutes les 10s, avec retry simple
    loop {
        // Revalider la connexion avant d’envoyer
        if let Err(e) = ensure_wifi_connected(&mut wifi) {
            eprintln!("⚠️ Wi-Fi KO: {e}. Retry dans 2s…");
            thread::sleep(Duration::from_secs(2));
            continue;
        }

        println!("📤 POST {url}");
        // Recréer la connexion HTTP à chaque tour pour éviter un client cassé
        let conn = EspHttpConnection::new(&HttpConfiguration::default())
            .context("Création HTTP client")?;
        let mut client = embedded_svc::http::client::Client::wrap(conn);

        // Envoi
        match (|| -> Result<u16> {
            let mut req = client.request(
                Method::Post,
                url,
                &[
                    ("Content-Type", "application/json"),
                    ("User-Agent", "mk2/0.1"),
                ],
            )?;
            req.write_all(br#"{"ping":true}"#)?;
            let resp = req.submit()?;
            Ok(resp.status())
        })() {
            Ok(status) => {
                println!("✅ Statut: {status}");
            }
            Err(e) => {
                eprintln!("❌ HTTP échec: {e}");
                // Si DNS/connexion foire: petite pause + on retentera
                thread::sleep(Duration::from_secs(3));
            }
        }

        thread::sleep(Duration::from_secs(10));
    }
}
