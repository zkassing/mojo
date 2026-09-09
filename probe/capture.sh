#!/bin/zsh
# 抓一段真实语音 → 自动解码 → 播放
# 用法: ./capture.sh [秒数]
#
# 实测参数（小米蓝牙语音遥控器，固件 2671）：
#   notification 120 字节裸 ADPCM，无帧头，解码器状态跨包连续，nibble 高→低
set -e
cd "$(dirname "$0")"
SECS=${1:-30}
RAW=/tmp/atvv_raw.bin
WAV=/tmp/atvv_voice.wav
LOG=/tmp/atvv_capture.log
rm -f "$RAW" "$WAV" "$LOG"

./build-gattprobe.sh >/dev/null

echo "▶︎ 采集窗口 ${SECS}s。看到「请按住语音键」后："
echo "   按住遥控器语音键 → 说 3-5 秒话 → 松开"
echo ""
open ./GattProbe.app --stdout "$LOG" --args \
  --handshake --dump-raw "$RAW" --timeout "$SECS"

# 跟踪日志，只显示关键行
( tail -f "$LOG" 2>/dev/null | grep --line-buffered -E "CAPS_RESP|AUDIO_START|AUDIO frame #1 |AUDIO_STOP|写入|没收到" ) &
TAILPID=$!
sleep $((SECS + 5))
kill $TAILPID 2>/dev/null || true

echo ""
echo "===== 采集结束 ====="
if [ ! -s "$RAW" ]; then
  echo "❌ 没抓到音频帧。检查 $LOG 里的 CTL 消息定位卡在哪一步。"
  exit 1
fi

FRAMES=$(( $(stat -f%z "$RAW") / 120 ))
echo "✅ 原始 ADPCM: $RAW ($(stat -f%z "$RAW") 字节 / ${FRAMES} 帧)"
echo ""
echo "▶︎ 解码…"
python3 decode_adpcm.py "$RAW" "$WAV" --swap-nibble

# 去直流 + 归一化，否则音量太小听不清
python3 - "$WAV" <<'PY'
import sys, wave, struct
p = sys.argv[1]
w = wave.open(p); n = w.getnframes(); rate = w.getframerate()
s = list(struct.unpack(f"<{n}h", w.readframes(n))); w.close()
dc = sum(s)/n
s = [x-dc for x in s]
peak = max(abs(x) for x in s) or 1
g = (32767*0.8)/peak
s = [max(-32768, min(32767, int(x*g))) for x in s]
o = wave.open(p,'wb'); o.setnchannels(1); o.setsampwidth(2); o.setframerate(rate)
o.writeframes(struct.pack(f"<{n}h", *s)); o.close()
print(f"  归一化增益 {g:.1f}x")
PY

echo ""
echo "▶︎ 播放 $WAV"
afplay "$WAV"
echo ""
echo "✅ 完成。wav 文件保留在 $WAV"
