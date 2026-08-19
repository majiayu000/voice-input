const scenes = {
  ready: {
    title: "本机输入法 · 就绪",
    hint: "按住 Fn 说话。松开后写入当前光标。",
    last: true,
    typed: "",
  },
  blocked: {
    title: "本机输入法 · 缺权限",
    hint: "辅助功能未开，写不进当前输入框。",
    last: false,
    typed: "",
  },
  error: {
    title: "本机输入法 · 出错",
    hint: "模型校验失败。已回到就绪，可重试。",
    last: false,
    typed: "",
  },
  listening: {
    hudTitle: "正在听",
    hudTime: "0:04",
    hudSub: "本机 · 松开 Fn 写入",
    typed: "",
  },
  recognizing: {
    hudTitle: "本机识别",
    hudTime: "—",
    hudSub: "第一次加载会稍慢",
    typed: "",
  },
  done: {
    hudTitle: "已写入",
    hudTime: "208 ms",
    hudSub: "未离开本机",
    typed: "本地语音链路已经贯通。",
  },
};

const onboardOrder = ["privacy", "permission", "model", "trial", "login"];

function setText(selector, value) {
  const node = document.querySelector(selector);
  if (node && value !== undefined) {
    node.textContent = value;
  }
}

function setScene(name) {
  document.body.dataset.scene = name;
  location.hash = name;
  document.querySelectorAll("[data-scene-link]").forEach((button) => {
    button.classList.toggle("is-on", button.dataset.sceneLink === name);
  });
  const copy = scenes[name];
  if (!copy) {
    return;
  }
  setText('[data-copy="title"]', copy.title);
  setText('[data-copy="hint"]', copy.hint);
  setText('[data-copy="hud-title"]', copy.hudTitle);
  setText('[data-copy="hud-time"]', copy.hudTime);
  setText('[data-copy="hud-sub"]', copy.hudSub);
  setText('[data-copy="typed"]', copy.typed);
  const last = document.querySelector('[data-copy="last"]');
  if (last && copy.last !== undefined) {
    last.classList.toggle("is-off", !copy.last);
  }
}

function setTab(name) {
  document.body.dataset.settings = name;
}

function setOnboard(name) {
  document.body.dataset.onboard = name;
  document.getElementById("onboard-next").textContent =
    name === "login" ? "完成" : "继续";
}

function shiftOnboard(delta) {
  const index = onboardOrder.indexOf(document.body.dataset.onboard);
  const next = onboardOrder[Math.min(onboardOrder.length - 1, Math.max(0, index + delta))];
  setOnboard(next);
}

document.querySelectorAll("[data-scene-link]").forEach((button) => {
  button.addEventListener("click", () => setScene(button.dataset.sceneLink));
});
document.querySelectorAll("[data-tab]").forEach((button) => {
  button.addEventListener("click", () => setTab(button.dataset.tab));
});
document.getElementById("onboard-back").addEventListener("click", () => shiftOnboard(-1));
document.getElementById("onboard-next").addEventListener("click", () => {
  if (document.body.dataset.onboard === "login") {
    setScene("ready");
    return;
  }
  shiftOnboard(1);
});
document.getElementById("tray-button").addEventListener("click", () => {
  const open = ["ready", "blocked", "error"].includes(document.body.dataset.scene);
  setScene(open ? "listening" : "ready");
});

if (location.search.includes("capture")) {
  document.body.classList.add("capture");
}

const hash = location.hash.replace("#", "") || "ready";
if (hash.startsWith("settings")) {
  const tab = hash.split("-")[1] || "general";
  setTab(["general", "asr", "text", "diag"].includes(tab) ? tab : "general");
  setScene("settings");
} else if (hash.startsWith("onboarding")) {
  const step = hash.split("-")[1] || "privacy";
  setOnboard(onboardOrder.includes(step) ? step : "privacy");
  setScene("onboarding");
} else {
  setOnboard("privacy");
  setScene(scenes[hash] ? hash : "ready");
}
