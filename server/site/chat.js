// The website's chat: new lines every few seconds, and sending without reloading the page.
// Without it the page still works: the form posts and the page comes back with the new line.
(() => {
  const box = document.querySelector("[data-chat]");
  if (!box) return;
  const api = box.dataset.chat;
  const me = (box.dataset.me || "").toLowerCase();
  const log = box.querySelector(".log");
  const form = box.querySelector(".say");
  let shown = "";

  const pad = (n) => String(n).padStart(2, "0");
  const line = (l) => {
    const li = document.createElement("li");
    if (l.from.toLowerCase() === me) li.className = "mine";
    const t = l.at % 86400;
    const time = document.createElement("span");
    time.className = "time";
    time.textContent = `${pad(Math.floor(t / 3600))}:${pad(Math.floor(t / 60) % 60)}`;
    if (l.medal) li.className = "medal";
    const who = document.createElement(l.medal ? "span" : "a");
    who.className = "player";
    if (!l.medal) who.href = `/u/${encodeURIComponent(l.from)}`;
    who.textContent = l.from;
    const text = document.createElement("span");
    text.className = "text";
    text.textContent = l.text;
    li.append(time, who, text);
    return li;
  };
  const render = (lines) => {
    const key = JSON.stringify(lines);
    if (key === shown) return;
    shown = key;
    const atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 40;
    log.replaceChildren(...lines.map(line));
    if (!lines.length) {
      const li = document.createElement("li");
      li.className = "dim empty";
      li.textContent = "No messages yet. Say hi.";
      log.append(li);
    }
    if (atBottom) log.scrollTop = log.scrollHeight;
  };
  const poll = async () => {
    if (document.hidden) return;
    try {
      const r = await fetch(api, { credentials: "same-origin" });
      if (r.ok) render(await r.json());
    } catch (_) {}
  };

  for (const li of log.querySelectorAll("li")) {
    const who = li.querySelector(".player");
    if (who && who.textContent.toLowerCase() === me) li.classList.add("mine");
  }
  log.scrollTop = log.scrollHeight;
  setInterval(poll, 3000);
  document.addEventListener("visibilitychange", poll);

  if (form) {
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const input = form.elements.text;
      const text = input.value.trim();
      if (!text) return;
      input.value = "";
      try {
        const r = await fetch(form.action + "?js=1", { method: "POST", body: new URLSearchParams({ text }), credentials: "same-origin" });
        const reply = await r.json();
        if (reply.error) {
          input.value = text;
          input.setCustomValidity(reply.error);
          input.reportValidity();
          setTimeout(() => input.setCustomValidity(""), 2500);
        }
      } catch (_) {
        input.value = text;
      }
      log.scrollTop = log.scrollHeight;
      poll();
    });
  }
})();
