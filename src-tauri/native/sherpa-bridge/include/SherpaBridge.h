#ifndef SHERPA_BRIDGE_H
#define SHERPA_BRIDGE_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/// 轻量桥：屏蔽 sherpa-onnx 配置结构细节，Swift 侧只面对 6 个函数
typedef struct MRSherpa MRSherpa;

/// 用模型目录创建识别器 + 一条流。model_dir 下需有
/// encoder/decoder/joiner 的 int8 onnx 与 tokens.txt。失败返回 NULL。
MRSherpa *mr_sherpa_create(const char *model_dir, int32_t num_threads);
void mr_sherpa_destroy(MRSherpa *p);

/// 喂 16kHz 单声道 float PCM（范围 -1..1），并推进解码
void mr_sherpa_accept(MRSherpa *p, const float *samples, int32_t n);

/// 当前识别文本（内部缓冲，下次 mr_sherpa_accept 前有效；无内容为 ""）
const char *mr_sherpa_text(MRSherpa *p);

/// 检测到句尾静音（一句话说完）
int32_t mr_sherpa_is_endpoint(MRSherpa *p);

/// 端点后开新的一句
void mr_sherpa_reset(MRSherpa *p);

/// 录音结束：冲刷尾包，之后可再取一次 text / is_endpoint
void mr_sherpa_input_finished(MRSherpa *p);

#ifdef __cplusplus
}
#endif

#endif
