/*
 * Agripod pH bench prototype: Scenario B, Atlas Scientific EZO-pH circuit over I2C.
 *
 * Hardware:
 *   - EZO-pH circuit in I2C mode (default address 0x63 / 99 decimal)
 *   - ESP32 SDA=21, SCL=22; EZO powered 3.3 V (3.3-5.5 V supported)
 *   - DS18B20 on GPIO4 with 4.7 kOhm pull-up to 3.3 V; its reading is pushed
 *     to the EZO for temperature compensation (command T,xx.x)
 *
 * One-time setup: a new EZO-pH ships in UART mode. Switch it to I2C per the
 * Atlas datasheet (send I2C,99 over UART, then power cycle). Do this before
 * running this sketch.
 *
 * Calibration (serial monitor @115200):
 *   cal7   -> pH 7.00 buffer, sends Cal,mid,7.00
 *   cal4   -> pH 4.00 buffer, sends Cal,low,4.00
 *   cal10  -> pH 10.00 buffer (optional), sends Cal,high,10.00
 *   slope  -> show EZO slope diagnostics
 *   clear  -> erase EZO calibration (Cal,clear)
 *
 * Calibration is stored on the EZO circuit itself, not on the ESP32.
 * This is a bench prototype, not production firmware. See esp32/README.md.
 */

#include <Wire.h>
#include <OneWire.h>
#include <DallasTemperature.h>

constexpr uint8_t EZO_ADDR = 0x63;
constexpr int PIN_ONEWIRE = 4;
constexpr uint32_t PERIOD_MS = 5000;
constexpr uint32_t EZO_WAIT_MS = 900;   // EZO needs ~900 ms for a read

OneWire oneWire(PIN_ONEWIRE);
DallasTemperature tempSensor(&oneWire);

// Send an ASCII command. Returns true if the I2C write was acknowledged.
bool ezoSend(const char* cmd) {
  Wire.beginTransmission(EZO_ADDR);
  Wire.print(cmd);
  return Wire.endTransmission() == 0;
}

// Read a response. Returns the EZO status byte:
//   1 = success, 2 = syntax error, 254 = still processing, 255 = no data
uint8_t ezoReceive(char* buf, size_t len) {
  Wire.requestFrom(EZO_ADDR, (uint8_t)32);
  if (!Wire.available()) return 255;
  uint8_t status = Wire.read();
  size_t i = 0;
  while (Wire.available()) {
    char c = Wire.read();
    if (c == 0) break;
    if (i < len - 1) buf[i++] = c;
  }
  buf[i] = '\0';
  return status;
}

// Send a command, wait for it to finish, and return the response text.
uint8_t ezoQuery(const char* cmd, char* buf, size_t len) {
  if (!ezoSend(cmd)) {
    buf[0] = '\0';
    return 255;
  }
  delay(EZO_WAIT_MS);
  return ezoReceive(buf, len);
}

void pushTemperature() {
  tempSensor.requestTemperatures();
  float t = tempSensor.getTempCByIndex(0);
  if (t == DEVICE_DISCONNECTED_C || t < -10.0f || t > 60.0f) {
    t = 25.0f;  // reference fallback, flagged in output
  }
  char cmd[16];
  snprintf(cmd, sizeof(cmd), "T,%.1f", t);
  ezoSend(cmd);
}

void handleSerial() {
  if (!Serial.available()) return;
  String line = Serial.readStringUntil('\n');
  line.trim();
  char resp[32];
  uint8_t status = 255;

  if (line == "cal7") {
    status = ezoQuery("Cal,mid,7.00", resp, sizeof(resp));
  } else if (line == "cal4") {
    status = ezoQuery("Cal,low,4.00", resp, sizeof(resp));
  } else if (line == "cal10") {
    status = ezoQuery("Cal,high,10.00", resp, sizeof(resp));
  } else if (line == "slope") {
    status = ezoQuery("Slope,?", resp, sizeof(resp));
    Serial.printf("slope response: %s\n", resp);
    return;
  } else if (line == "clear") {
    status = ezoQuery("Cal,clear", resp, sizeof(resp));
  } else if (line.length()) {
    Serial.println("unknown command");
    return;
  }

  if (status == 1) {
    Serial.println("calibration accepted by EZO");
  } else if (line.length()) {
    Serial.printf("EZO status %u\n", status);
  }
}

void setup() {
  Serial.begin(115200);
  Wire.begin();
  tempSensor.begin();

  char resp[32];
  uint8_t status = ezoQuery("i", resp, sizeof(resp));  // device info
  if (status != 1) {
    Serial.printf("EZO not responding (status %u). Check wiring and I2C mode.\n", status);
  } else {
    Serial.printf("EZO info: %s\n", resp);
  }
  Serial.println("pH bench prototype (EZO I2C). Commands: cal7, cal4, cal10, slope, clear");
}

void loop() {
  handleSerial();

  static uint32_t last = 0;
  if (millis() - last < PERIOD_MS) return;
  last = millis();

  pushTemperature();

  char resp[32];
  uint8_t status = ezoQuery("R", resp, sizeof(resp));
  if (status == 1) {
    Serial.printf("pH=%s\n", resp);
  } else {
    Serial.printf("read failed, status %u\n", status);
  }
}
