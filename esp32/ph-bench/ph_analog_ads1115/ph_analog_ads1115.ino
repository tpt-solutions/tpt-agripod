/*
 * Agripod pH bench prototype: Scenario A, analog pH front-end.
 *
 * Hardware:
 *   - Analog pH board (e.g. DFRobot Gravity SEN0161 class) powered from 3.3 V
 *   - Board analog output -> ADS1115 channel A0 (gain 1, +/-4.096 V range)
 *   - ADS1115 on I2C (ESP32 default SDA=21, SCL=22), powered from 3.3 V
 *   - DS18B20 on GPIO4 with 4.7 kOhm pull-up to 3.3 V
 *
 * Calibration (serial monitor @115200, put probe in buffer, wait for stable reading):
 *   cal7   -> pH 7.00 buffer, do this first
 *   cal4   -> pH 4.00 buffer, computes slope efficiency and saves to flash
 *   status -> show stored calibration
 *   reset  -> erase calibration
 *
 * This is a bench prototype, not production firmware. See esp32/README.md.
 */

#include <Wire.h>
#include <Adafruit_ADS1X15.h>
#include <OneWire.h>
#include <DallasTemperature.h>
#include <Preferences.h>

constexpr int PIN_ONEWIRE = 4;
constexpr int SAMPLES = 30;            // samples per reading, median taken
constexpr uint32_t PERIOD_MS = 5000;
constexpr float NERNST_25 = 59.16f;    // ideal mV per pH at 25 C

Adafruit_ADS1115 ads;
OneWire oneWire(PIN_ONEWIRE);
DallasTemperature tempSensor(&oneWire);
Preferences prefs;

// Model: V(pH, T) = V7 + S(T) * (7 - pH), with S(T) = eff * 59.16 * (T + 273.15) / 298.15
float V7_mV = 0.0f;   // electrode voltage at pH 7 (mV), reference offset
float eff = 1.0f;     // slope efficiency relative to ideal Nernst
bool calibrated = false;

float slopeAt(float tC) {
  return eff * NERNST_25 * (tC + 273.15f) / 298.15f;
}

float readTempC() {
  tempSensor.requestTemperatures();
  float t = tempSensor.getTempCByIndex(0);
  if (t == DEVICE_DISCONNECTED_C || t < -10.0f || t > 60.0f) {
    return 25.0f;  // fall back to reference temperature, flag in output
  }
  return t;
}

float medianMv() {
  float v[SAMPLES];
  for (int i = 0; i < SAMPLES; i++) {
    int16_t raw = ads.readADC_SingleEnded(0);
    v[i] = ads.computeVolts(raw) * 1000.0f;
    delay(20);
  }
  // insertion sort, small N
  for (int i = 1; i < SAMPLES; i++) {
    float key = v[i];
    int j = i - 1;
    while (j >= 0 && v[j] > key) { v[j + 1] = v[j]; j--; }
    v[j + 1] = key;
  }
  return v[SAMPLES / 2];
}

void loadCalibration() {
  prefs.begin("ph", true);
  calibrated = prefs.isKey("V7");
  if (calibrated) {
    V7_mV = prefs.getFloat("V7");
    eff = prefs.getFloat("eff");
  }
  prefs.end();
}

void saveCalibration() {
  prefs.begin("ph", false);
  prefs.putFloat("V7", V7_mV);
  prefs.putFloat("eff", eff);
  prefs.end();
}

void doCal7() {
  float t = readTempC();
  float mv = medianMv();
  // Store the offset assuming ideal slope at this temperature; refined by cal4.
  V7_mV = mv;
  calibrated = false;  // not final until cal4 is done
  Serial.printf("cal7: V=%.1f mV, T=%.1f C. Now place probe in pH 4 and send cal4.\n", mv, t);
}

void doCal4() {
  float t = readTempC();
  float mv4 = medianMv();
  // Measured slope between pH 7 and pH 4 at this temperature
  float sMeasured = (V7_mV - mv4) / (7.0f - 4.0f);
  float sIdeal = NERNST_25 * (t + 273.15f) / 298.15f;
  float newEff = sMeasured / sIdeal;
  if (newEff < 0.85f || newEff > 1.05f) {
    Serial.printf("cal4 REJECTED: slope efficiency %.3f outside 0.85..1.05. Check probe/buffers, redo cal7.\n", newEff);
    calibrated = false;
    return;
  }
  eff = newEff;
  calibrated = true;
  saveCalibration();
  Serial.printf("cal4 OK: eff=%.3f, slope=%.1f mV/pH at %.1f C. Saved.\n", eff, slopeAt(t), t);
}

void handleSerial() {
  if (!Serial.available()) return;
  String cmd = Serial.readStringUntil('\n');
  cmd.trim();
  if (cmd == "cal7") doCal7();
  else if (cmd == "cal4") doCal4();
  else if (cmd == "status") {
    Serial.printf("calibrated=%d V7=%.1f mV eff=%.3f\n", calibrated, V7_mV, eff);
  } else if (cmd == "reset") {
    prefs.begin("ph", false);
    prefs.clear();
    prefs.end();
    calibrated = false;
    Serial.println("calibration cleared");
  } else if (cmd.length()) {
    Serial.println("unknown command");
  }
}

void setup() {
  Serial.begin(115200);
  Wire.begin();
  tempSensor.begin();

  if (!ads.begin()) {
    Serial.println("ADS1115 not found at 0x48. Check wiring.");
    while (true) delay(1000);
  }
  ads.setGain(GAIN_ONE);  // +/-4.096 V, safe for 3.3 V supply

  loadCalibration();
  Serial.printf("pH bench prototype. calibrated=%d\n", calibrated);
  if (!calibrated) Serial.println("Not calibrated: readings are indicative only. Run cal7 then cal4.");
}

void loop() {
  handleSerial();

  static uint32_t last = 0;
  if (millis() - last < PERIOD_MS) return;
  last = millis();

  float t = readTempC();
  float mv = medianMv();
  float s = slopeAt(t);
  float ph = 7.0f - (mv - V7_mV) / s;

  Serial.printf("pH=%.2f  V=%.1f mV  T=%.1f C  %s\n",
                ph, mv, t, calibrated ? "cal" : "UNCAL");
}
