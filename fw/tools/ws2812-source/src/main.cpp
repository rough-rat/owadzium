#include <Arduino.h>
#include <driver/rmt.h>

#ifndef WS2812_PIN
#define WS2812_PIN 18
#endif

// Hardware RMT, 80 MHz / 2 = 40 MHz: 50 ticks per 800 kHz bit.
// 0 = 400 ns high, 850 ns low; 1 = 800 ns high, 450 ns low.
static rmt_item32_t items[16 * 24 + 1];
static uint8_t pixels[16][3]; // GRB
static unsigned pixelCount = 16;
static RingbufHandle_t received;
static unsigned rxFrames, rxBad;
static unsigned frame;
static char mode = 'a';

static void sendFrame() {
    unsigned pos = 0;
    for (unsigned i = 0; i < pixelCount; ++i) {
        auto &pixel = pixels[i];
        for (uint8_t component : pixel) {
            for (int bit = 7; bit >= 0; --bit) {
                bool one = component & (1 << bit);
                items[pos].level0 = 1;
                items[pos].duration0 = one ? 32 : 16;
                items[pos].level1 = 0;
                items[pos++].duration1 = one ? 18 : 34;
            }
        }
    }
    items[pos].level0 = items[pos].level1 = 0;
    items[pos].duration0 = items[pos].duration1 = 6000; // 300 us reset
    ESP_ERROR_CHECK(rmt_write_items(RMT_CHANNEL_0, items, pos + 1, true));
}

void setup() {
    Serial.begin(115200);
    rmt_config_t config = RMT_DEFAULT_CONFIG_TX((gpio_num_t)WS2812_PIN, RMT_CHANNEL_0);
    config.clk_div = 2;
    config.mem_block_num = 4;
    ESP_ERROR_CHECK(rmt_config(&config));
    ESP_ERROR_CHECK(rmt_driver_install(RMT_CHANNEL_0, 0, 0));
    rmt_config_t rx = RMT_DEFAULT_CONFIG_RX(GPIO_NUM_19, RMT_CHANNEL_4);
    rx.clk_div = 2;
    rx.mem_block_num = 4;
    rx.rx_config.filter_ticks_thresh = 4;
    rx.rx_config.idle_threshold = 4000; // 100 us at 40 MHz
    ESP_ERROR_CHECK(rmt_config(&rx));
    ESP_ERROR_CHECK(rmt_driver_install(RMT_CHANNEL_4, 4096, 0));
    ESP_ERROR_CHECK(rmt_get_ringbuf_handle(RMT_CHANNEL_4, &received));
    ESP_ERROR_CHECK(rmt_rx_start(RMT_CHANNEL_4, true));
    Serial.printf("WS2812 source GPIO%d, 16 GRB pixels; chain return GPIO19, 50 fps\n", WS2812_PIN);
    Serial.println("a=automatic, 0=off, 1=on, c=chase, r/g/b=component, t=test, 8=8 pixels, d=16 pixels");
}

static void checkReturn() {
    size_t size = 0;
    auto data = (rmt_item32_t *)xRingbufferReceive(received, &size, 0);
    if (!data) return;
    uint8_t decoded[24] = {};
    unsigned bits = 0;
    unsigned minHigh = 32767, maxHigh = 0;
    for (unsigned i = 0; i < size / sizeof(*data); ++i) {
        unsigned levels[] = {data[i].level0, data[i].level1};
        unsigned ticks[] = {data[i].duration0, data[i].duration1};
        for (unsigned j = 0; j < 2; ++j) {
            if (!levels[j] || !ticks[j]) continue;
            minHigh = min(minHigh, ticks[j]);
            maxHigh = max(maxHigh, ticks[j]);
            if (bits < 192) decoded[bits / 8] = (decoded[bits / 8] << 1) | (ticks[j] > 24);
            ++bits;
        }
    }
    bool ok = pixelCount == 16 && bits == 192 && memcmp(decoded, pixels[8], 24) == 0;
    ++rxFrames;
    if (!ok) ++rxBad;
    // Every fixed-pattern result can be inspected independently of local LEDs.
    if (frame % 50 == 0 || !ok) {
        Serial.printf("return frames=%u bad=%u bits=%u high_ticks=%u..%u data=", rxFrames, rxBad, bits, minHigh, maxHigh);
        for (auto byte : decoded) Serial.printf("%02x", byte);
        Serial.println(ok ? " PASS" : " FAIL");
    }
    vRingbufferReturnItem(received, data);
    ESP_ERROR_CHECK(rmt_rx_start(RMT_CHANNEL_4, true));
}

void loop() {
    while (Serial.available()) {
        char input = Serial.read();
        if (input == '8' || input == 'd') {
            pixelCount = input == '8' ? 8 : 16;
            Serial.printf("pixels %u\n", pixelCount);
        }
        if (strchr("a01crgbt", input)) {
            mode = input;
            Serial.printf("mode %c\n", mode);
        }
    }
    // Each phase lasts two seconds: chase, ramp, off, on, G, R, B, test bytes.
    char selected = mode == 'a' ? "c-01grbt"[(frame / 100) % 8] : mode;
    memset(pixels, 0, sizeof(pixels));
    static const uint8_t test[] = {0, 1, 128, 255, 51, 254, 170, 3};
    for (unsigned i = 0; i < pixelCount; ++i) {
        switch (selected) {
        case 'c':
            if (i == (frame / 8) % pixelCount) pixels[i][0] = 255;
            break;
        case '-':
            pixels[i][0] = (frame * 3 + i * 32) & 255;
            break;
        case '1': memset(pixels[i], 255, 3); break;
        case 'g': pixels[i][0] = 255; break;
        case 'r': pixels[i][1] = 255; break;
        case 'b': pixels[i][2] = 255; break;
        case 't': pixels[i][i % 3] = test[i < 8 ? i : 15 - i]; break;
        }
    }
    sendFrame();
    delay(20);
    checkReturn();
    ++frame;
}
