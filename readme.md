# 🌡️ ESP32-C3 Wi-Fi Setup + DHT11 Sensor + HTTP POST

This project turns your **ESP32-C3 Mini** into a smart sensor that:

1. Starts as a **Wi-Fi Access Point (AP)**
2. Hosts a **local setup server** to collect your Wi-Fi credentials
3. Connects automatically to your network
4. Reads **temperature & humidity** from a **DHT11 sensor**
5. Sends data via **HTTP POST** to a remote endpoint every 10 seconds

---

## 🧠 Features

✅ Local configuration via captive AP  
✅ Wi-Fi STA connection with DHCP wait logic  
✅ DHT11 sensor readings (temperature + humidity)  
✅ JSON payloads sent to remote HTTP endpoint  
✅ Robust URL decoding and safe string handling  
✅ Error logging with `anyhow` + `esp-idf-svc`

---

## 🧰 Dependencies

Add these dependencies to your `Cargo.toml`:

```toml
[dependencies]
anyhow = "1"
embedded-svc = "0.27"
esp-idf-svc = "0.48"
esp-idf-hal = "0.48"
esp-idf-sys = "0.48"
dht-sensor = "0.2"
log = "0.4"
```

> ⚙️ Make sure your Rust toolchain is configured for ESP32 development:
>
> ```bash
> rustup target add riscv32imc-esp-espidf
> cargo install espflash ldproxy
> ```

---

## ⚙️ Wiring

| Pin   | Component   | Description |
|--------|-------------|-------------|
| GPIO4  | DHT11 Data  | Data pin    |
| 3V3    | DHT11 VCC   | Power       |
| GND    | DHT11 GND   | Ground      |

> The code uses GPIO 4 by default:
> ```rust
> let mut pin = PinDriver::input_output_od(peripherals.pins.gpio4)?;
> ```

---

## 🚀 How It Works

### 🟢 1. Start Access Point

On boot, the ESP starts an open Wi-Fi network:

```
SSID: ESP32_SETUP
URL:  http://192.168.71.1/
```

Connect to it with your phone or computer.

---

### 🖥️ 2. Local Setup Page

When connected, open `http://192.168.71.1/` in your browser.

You’ll see a small form:

```
SSID: [_________]
Password: [_________]
[ Connect ]
```

Submitting sends your credentials via POST `/setup`.

---

### 📡 3. Connect to Home Wi-Fi

Once received, the ESP disconnects from AP mode and connects to your Wi-Fi as a **station (STA)**.
It waits up to **20 seconds** for DHCP to assign an IP.

---

### 🌡️ 4. DHT11 Sensor Loop

After connection, it continuously reads temperature and humidity:

```
Temp: 25 °C, Humidity: 48 %
```

---

### 🌍 5. HTTP POST to Server

Every 10 seconds, it sends data like this:

```json
{
  "ping": true,
  "temperature": 25,
  "humidity": 48
}
```

The endpoint is defined in the code:
```rust
let url = "http://b15ca8fb2839.ngrok-free.app/ping";
```

---

## 🧾 Example Output

```
📡 AP 'ESP32_SETUP' started → http://192.168.71.1/
🖥️ Waiting for Wi-Fi credentials...
📡 Received setup: ssid='MyHomeWiFi', pass_len=10
📡 Connecting to 'MyHomeWiFi'
✅ Got IP: 192.168.0.24
✅ Connected! Starting ADC read loop...
Temp: 23 °C, Humidity: 51 %
📨 Status: 200
```

---

## 🧩 Customization

| Feature       | Function / Section        | Default |
|----------------|----------------------------|----------|
| AP SSID        | `start_ap()`              | `ESP32_SETUP` |
| DHT11 Pin      | `gpio4`                   | change via `PinDriver` |
| HTTP Endpoint  | `main()`                  | `http://b15ca8fb2839.ngrok-free.app/ping` |
| Loop Interval  | `thread::sleep()`         | 10 seconds |

---

## 🧪 Debugging

If Wi-Fi doesn’t connect:

- Check your power supply (stable 5V/USB)
- Increase DHCP timeout in `wait_for_ip()`
- Use `log::info!` for extra details
- Ensure SSID/password have no invalid characters

---

## 🛠️ Build & Flash

```bash
cargo build --release
espflash flash target/riscv32imc-esp-espidf/release/esp32_wifi_dht
```

Or simply:

```bash
espflash flash --monitor
```

---

## 📜 License

MIT License © 2025 Yassine Hadi
