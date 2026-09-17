import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { toast } from "sonner";

/**
 * 检查并应用更新。
 * - silent=true（启动时自动检查）：无更新/网络出错都不打扰，仅在发现新版本时提示。
 * - silent=false（服务页手动检查）：把结果明确反馈给用户。
 * 返回是否发现了新版本。
 */
export async function checkForUpdate(silent = false): Promise<boolean> {
  let update: Awaited<ReturnType<typeof check>> = null;
  try {
    update = await check();
  } catch (e) {
    if (!silent) toast.error(`检查更新失败：${String(e)}`);
    return false;
  }

  if (!update?.available) {
    if (!silent) toast.success("已是最新版本");
    return false;
  }

  const version = update.version;
  const toastId = toast.message(`发现新版本 v${version}，开始下载… 0%`, {
    duration: Infinity,
  });
  let total = 0;
  let downloaded = 0;

  try {
    await update.downloadAndInstall((e) => {
      switch (e.event) {
        case "Started":
          total = typeof e.data.contentLength === "number" ? e.data.contentLength : 0;
          downloaded = 0;
          toast.message(`正在下载 v${version} 0%`, {
            id: toastId,
            duration: Infinity,
          });
          break;
        case "Progress":
          downloaded += e.data.chunkLength;
          if (total > 0) {
            const pct = Math.min(100, Math.round((downloaded / total) * 100));
            toast.message(`正在下载 v${version} ${pct}%`, {
              id: toastId,
              duration: Infinity,
            });
          }
          break;
        case "Finished":
          toast.message("下载完成，准备安装…", {
            id: toastId,
            duration: Infinity,
          });
          break;
      }
    });
  } catch (e) {
    toast.error(`下载更新失败：${String(e)}`, { id: toastId });
    return false;
  }

  toast.success("更新已就绪，重启后生效", {
    id: toastId,
    duration: 8000,
    action: {
      label: "立即重启",
      onClick: () => {
        void relaunch();
      },
    },
  });
  return true;
}
