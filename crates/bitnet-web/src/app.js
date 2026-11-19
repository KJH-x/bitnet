"use strict";

const element = (id) => document.getElementById(id);
const labels = {
  zh: {
    title: "BitNet 日志", search: "筛选", refresh: "刷新", export: "导出 JSON", copy: "复制 JSON", previous: "上一页",
    next: "下一页", stop: "停止服务", language: "English", loading: "正在读取日志…",
    unavailable: "无法读取日志。请检查认证引擎是否正在运行，然后刷新。",
    expired: "日志会话已结束，请从 BitNet 重新打开。",
    stopped: "日志 Web 服务已停止。", stopping: "正在停止日志服务…",
    stopFailed: "未能确认服务已停止。", noMatches: "没有匹配的记录。",
    copied: "已复制匹配的记录。", copyFailed: "无法访问剪贴板。可用「导出 JSON」保存记录。",
    earlier: "加载更早记录", earlierFailed: "无法读取更早的记录。",
    result: (visible, total) => `${visible} 条匹配，共加载最近 ${total} 条记录。`,
  },
  en: {
    title: "BitNet activity", search: "Filter", refresh: "Refresh", export: "Export JSON", copy: "Copy JSON",
    previous: "Previous", next: "Next", stop: "Stop viewer", language: "简体中文",
    loading: "Loading activity…", unavailable: "Activity is unavailable. Check that the engine is running, then refresh.",
    expired: "This viewer session ended. Reopen it from BitNet.",
    stopped: "The log viewer has stopped.", stopping: "Stopping the log viewer…",
    stopFailed: "Could not confirm that the viewer stopped.", noMatches: "No matching records.",
    copied: "Matching records copied.", copyFailed: "Clipboard unavailable. Use Export JSON to save the records.",
    earlier: "Load earlier activity", earlierFailed: "Could not read earlier activity.",
    result: (visible, total) => `${visible} matches in the ${total} latest records loaded.`,
  },
};

const chineseKinds = {
  reconnect: "会话重连成功", login_success: "校园网登录成功", login_failure: "校园网登录失败",
  environment_observed: "网络状态已更新", online_segment: "在线时段已记录",
  session_lost: "认证失效，等待重连", health_degraded: "认证有效，网络异常",
};
const englishKinds = {
  reconnect: "Connection restored", login_success: "Signed in to the campus network",
  login_failure: "Campus sign-in failed", environment_observed: "Network status updated",
  online_segment: "Online period recorded", session_lost: "Signed out, waiting to reconnect",
  health_degraded: "Signed in, but network checks failed",
};
const incoming = location.hash.slice(1);
let token = /^[a-f0-9]{64}$/.test(incoming) ? incoming : "";
try {
  if (token) sessionStorage.setItem("bitnet-view", token);
  else token = sessionStorage.getItem("bitnet-view") || "";
} catch {}
if (incoming) history.replaceState(null, "", location.pathname + location.search);

let language = new URLSearchParams(location.search).get("lang") === "en" ? "en" : "zh";
let rows = [];
let page = 0;
let state = token ? "loading" : "expired";
const pageSize = 20;
const batchSize = 40;
let more = false;
let pending = false;
const escapedText = (value) => typeof value === "string" ? value : "";
const title = (kind) => {
  const kinds = language === "zh" ? chineseKinds : englishKinds;
  return Object.hasOwn(kinds, kind) ? kinds[kind] : escapedText(kind).replaceAll("_", " ");
};
const matches = () => {
  const query = element("search").value.trim().toLowerCase();
  return query ? rows.filter((row) =>
    [row.kind, title(row.kind), row.message, row.account].some((text) => escapedText(text).toLowerCase().includes(query))
  ) : rows;
};

function render() {
  const t = labels[language];
  document.title = t.title;
  document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
  for (const [id, key] of Object.entries({ title: "title", "search-label": "search", language: "language",
    stop: "stop", refresh: "refresh", export: "export", copy: "copy", previous: "previous", next: "next",
    earlier: "earlier" })) {
    element(id).textContent = t[key];
  }
  const records = matches();
  const pages = Math.max(1, Math.ceil(records.length / pageSize));
  page = Math.min(page, pages - 1);
  const unavailable = state === "expired" || state === "stopped" || state === "stopping";
  element("refresh").disabled = unavailable || pending;
  element("stop").disabled = unavailable;
  element("previous").disabled = page === 0;
  element("next").disabled = page + 1 === pages;
  element("export").disabled = records.length === 0;
  element("copy").disabled = records.length === 0;
  element("earlier").hidden = unavailable || !more;
  element("earlier").disabled = pending;
  element("page").textContent = `${page + 1} / ${pages}`;
  element("notice").textContent = state === "ready"
    ? (records.length ? t.result(records.length, rows.length) : t.noMatches)
    : t[state];
  const list = element("entries");
  const expanded = new Set(Array.from(list.querySelectorAll("details[open]"), (item) => item.dataset.id));
  list.replaceChildren();
  for (const row of records.slice(page * pageSize, (page + 1) * pageSize)) {
    const entry = document.createElement("details");
    entry.dataset.id = row.id;
    entry.open = expanded.has(entry.dataset.id);
    const summary = document.createElement("summary");
    const name = document.createElement("strong");
    name.textContent = title(row.kind);
    const time = document.createElement("time");
    const date = new Date(row.at * 1000);
    if (!Number.isNaN(date.valueOf())) {
      const minutes = Math.max(0, Math.floor((Date.now() - date.valueOf()) / 60000));
      time.dateTime = date.toISOString();
      time.title = date.toLocaleString(document.documentElement.lang);
      time.textContent = language === "zh"
        ? (minutes === 0 ? "刚刚" : `${minutes} 分钟前`)
        : (minutes === 0 ? "Just now" : `${minutes} minute${minutes === 1 ? "" : "s"} ago`);
    }
    summary.append(name, time);
    const message = document.createElement("p");
    message.textContent = escapedText(row.message);
    const json = document.createElement("pre");
    json.textContent = JSON.stringify(row, null, 2);
    entry.append(summary, message, json);
    list.append(entry);
  }
}

const authorize = () => ({ Authorization: `Bearer ${token}` });
async function refresh({ watch = false, earlier = false } = {}) {
  if (pending || !token || state === "stopping" || state === "stopped" || state === "expired") return;
  const before = earlier ? rows.at(-1)?.id : undefined;
  if (earlier && !before) return;
  const suffix = earlier ? `?before=${encodeURIComponent(before)}` : watch ? "?watch" : "";
  pending = true;
  if (!watch) render();
  let note;
  let changed = !watch;
  try {
    const response = await fetch(`/api/history${suffix}`, { headers: authorize(), cache: "no-store" });
    if (state === "stopping" || state === "stopped") return;
    if (response.status === 401) {
      state = "expired";
      changed = true;
    } else if (!response.ok) throw new Error("history unavailable");
    else {
      const data = await response.json();
      if (state === "stopping" || state === "stopped") return;
      if (!Array.isArray(data)) throw new Error("invalid response");
      if (earlier) {
        rows = rows.concat(data);
        more = data.length === batchSize;
      } else if (!watch) {
        rows = data;
        more = data.length === batchSize;
        page = 0;
      } else if (data[0]?.id !== rows[0]?.id) {
        const recent = new Set(data.map((row) => row.id));
        if (rows.some((row) => recent.has(row.id))) {
          rows = data.concat(rows.filter((row) => !recent.has(row.id)));
        } else {
          rows = data;
          more = data.length === batchSize;
        }
        changed = true;
      }
      changed ||= state !== "ready";
      state = "ready";
    }
  } catch {
    if (state === "stopping" || state === "stopped") return;
    if (earlier) note = labels[language].earlierFailed;
    else if (!watch || rows.length === 0) { state = "unavailable"; changed = true; }
  } finally {
    pending = false;
    if (changed) render();
    if (note) element("notice").textContent = note;
  }
}

element("language").addEventListener("click", () => { language = language === "zh" ? "en" : "zh"; render(); });
element("search").addEventListener("input", () => { page = 0; render(); });
element("previous").addEventListener("click", () => { if (page > 0) page--; render(); });
element("next").addEventListener("click", () => { page++; render(); });
element("refresh").addEventListener("click", () => refresh());
element("earlier").addEventListener("click", () => refresh({ earlier: true }));
element("copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(JSON.stringify(matches(), null, 2));
    element("notice").textContent = labels[language].copied;
  } catch {
    element("notice").textContent = labels[language].copyFailed;
  }
});
element("export").addEventListener("click", () => {
  const url = URL.createObjectURL(new Blob([JSON.stringify(matches(), null, 2)], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = "bitnet-activity.json";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
});
element("stop").addEventListener("click", async () => {
  state = "stopping";
  render();
  try {
    const response = await fetch("/api/stop", { method: "POST", headers: authorize() });
    if (!response.ok && response.status !== 401) throw new Error("stop failed");
    state = response.ok ? "stopped" : "expired";
    rows = [];
    token = "";
    try { sessionStorage.removeItem("bitnet-view"); } catch {}
  } catch {
    state = "stopFailed";
  }
  render();
});
render();
if (token) refresh();
setInterval(() => {
  if (!document.hidden && page === 0 && window.scrollY < 32 && !element("search").value) {
    refresh({ watch: true });
  }
}, 5000);
