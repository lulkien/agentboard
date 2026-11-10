(() => {
  const body = document.body;
  const status = document.querySelector("#pulse-status");
  const message = document.querySelector("[data-pulse-message]");
  const followButton = document.querySelector("[data-follow]");
  if (!body || !status || !message || !followButton) return;

  let cursor = body.dataset.cursor || "";
  let failures = 0;
  let follow = false;
  try {
    follow = localStorage.getItem("agentboard-follow") === "true";
  } catch (_) {}
  followButton.setAttribute("aria-pressed", String(follow));

  function show(text, reload) {
    message.textContent = text;
    const oldLink = status.querySelector("a");
    if (oldLink) oldLink.remove();
    if (reload) {
      const link = document.createElement("a");
      link.href = location.href;
      link.textContent = "Reload";
      status.append(link);
    }
  }

  function focusedFormField() {
    return document.activeElement && document.activeElement.matches("input, select, textarea");
  }

  followButton.addEventListener("click", () => {
    follow = !follow;
    followButton.setAttribute("aria-pressed", String(follow));
    try {
      localStorage.setItem("agentboard-follow", String(follow));
    } catch (_) {}
  });

  async function poll() {
    let delay = 5000;
    if (!document.hidden) {
      try {
        const response = await fetch("/pulse", { cache: "no-store" });
        if (!response.ok) throw new Error("pulse request failed");
        const next = (await response.text()).trim();
        failures = 0;
        const [instance, rawSeq] = next.split(":");
        const [oldInstance, rawOldSeq] = cursor.split(":");
        const seq = Number(rawSeq);
        const oldSeq = Number(rawOldSeq);
        if (instance !== oldInstance) {
          show("Board was reset", true);
        } else if (Number.isFinite(seq) && seq > oldSeq) {
          status.classList.add("pulse-new");
          show(`${seq - oldSeq} new events`, true);
          if (follow && !focusedFormField()) location.reload();
        }
        cursor = next;
      } catch (_) {
        failures += 1;
        if (failures >= 2) {
          show("Dashboard not reachable", false);
          delay = 30000;
        }
      }
    }
    window.setTimeout(poll, delay);
  }

  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) poll();
  });
  window.setTimeout(poll, 5000);
})();
