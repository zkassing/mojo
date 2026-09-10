#include "SherpaBridge.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "sherpa-onnx/c-api/c-api.h"

struct MRSherpa {
    const SherpaOnnxOnlineRecognizer *recognizer;
    const SherpaOnnxOnlineStream *stream;
    char text[4096];
};

/// 按优先级探测模型文件（兼容 X-ASR 命名与旧命名）
static int pick(char *out, size_t n, const char *dir, const char *a, const char *b) {
    const char *names[2] = {a, b};
    for (int i = 0; i < 2; i++) {
        snprintf(out, n, "%s/%s", dir, names[i]);
        FILE *f = fopen(out, "rb");
        if (f) { fclose(f); return 1; }
    }
    return 0;
}

MRSherpa *mr_sherpa_create(const char *model_dir, int32_t num_threads) {
    char enc[2048], dec[2048], joiner[2048], tokens[2048];
    if (!pick(enc, sizeof(enc), model_dir, "encoder.int8.onnx", "encoder-epoch-99-avg-1.int8.onnx"))
        return NULL;
    if (!pick(dec, sizeof(dec), model_dir, "decoder.int8.onnx", "decoder-epoch-99-avg-1.int8.onnx")) {
        // X-ASR 的 decoder 无量化版
        if (!pick(dec, sizeof(dec), model_dir, "decoder.onnx", "decoder-epoch-99-avg-1.onnx"))
            return NULL;
    }
    if (!pick(joiner, sizeof(joiner), model_dir, "joiner.int8.onnx", "joiner-epoch-99-avg-1.int8.onnx"))
        return NULL;
    snprintf(tokens, sizeof(tokens), "%s/tokens.txt", model_dir);

    SherpaOnnxOnlineRecognizerConfig config;
    memset(&config, 0, sizeof(config));
    config.feat_config.sample_rate = 16000;
    config.feat_config.feature_dim = 80;
    config.model_config.transducer.encoder = enc;
    config.model_config.transducer.decoder = dec;
    config.model_config.transducer.joiner = joiner;
    config.model_config.tokens = tokens;
    config.model_config.num_threads = num_threads > 0 ? num_threads : 2;
    config.model_config.provider = "cpu";
    // 不设 model_type：让运行时从模型 metadata 自动识别
    // （旧双语模型是 zipformer，新 X-ASR 模型元数据键不同，硬编码会加载失败）
    config.enable_endpoint = 1;
    config.rule1_min_trailing_silence = 2.4f;
    config.rule2_min_trailing_silence = 1.2f;
    config.rule3_min_utterance_length = 20.0f;

    const SherpaOnnxOnlineRecognizer *r = SherpaOnnxCreateOnlineRecognizer(&config);
    if (!r) return NULL;

    MRSherpa *m = (MRSherpa *)calloc(1, sizeof(MRSherpa));
    m->recognizer = r;
    m->stream = SherpaOnnxCreateOnlineStream(r);
    if (!m->stream) {
        SherpaOnnxDestroyOnlineRecognizer(r);
        free(m);
        return NULL;
    }
    return m;
}

void mr_sherpa_destroy(MRSherpa *p) {
    if (!p) return;
    SherpaOnnxDestroyOnlineStream(p->stream);
    SherpaOnnxDestroyOnlineRecognizer(p->recognizer);
    free(p);
}

/// 推进解码并把最新文本拷进内部缓冲
static void pump(MRSherpa *p) {
    while (SherpaOnnxIsOnlineStreamReady(p->recognizer, p->stream)) {
        SherpaOnnxDecodeOnlineStream(p->recognizer, p->stream);
    }
    const SherpaOnnxOnlineRecognizerResult *r =
        SherpaOnnxGetOnlineStreamResult(p->recognizer, p->stream);
    snprintf(p->text, sizeof(p->text), "%s", (r && r->text) ? r->text : "");
    if (r) SherpaOnnxDestroyOnlineRecognizerResult(r);
}

void mr_sherpa_accept(MRSherpa *p, const float *samples, int32_t n) {
    if (!p || n <= 0) return;
    SherpaOnnxOnlineStreamAcceptWaveform(p->stream, 16000, samples, n);
    pump(p);
}

const char *mr_sherpa_text(MRSherpa *p) { return p ? p->text : ""; }

int32_t mr_sherpa_is_endpoint(MRSherpa *p) {
    return p ? SherpaOnnxOnlineStreamIsEndpoint(p->recognizer, p->stream) : 0;
}

void mr_sherpa_reset(MRSherpa *p) {
    if (p) SherpaOnnxOnlineStreamReset(p->recognizer, p->stream);
}

void mr_sherpa_input_finished(MRSherpa *p) {
    if (!p) return;
    // 冲刷尾部上下文：streaming 模型需要约 1s 以上的尾部音频才能把最后
    // 几个字吐出来（实测 480ms 延迟模型不垫静音会丢尾词）
    static float zeros[16000];
    SherpaOnnxOnlineStreamAcceptWaveform(p->stream, 16000, zeros, 16000);
    SherpaOnnxOnlineStreamAcceptWaveform(p->stream, 16000, zeros, 8000);
    SherpaOnnxOnlineStreamInputFinished(p->stream);
    pump(p);
}
