const BUTTON_ID = "ddockit-run-button";

function ensureButton() {
  if (document.getElementById(BUTTON_ID)) {
    return;
  }

  const toolbar = document.querySelector(".file-navigation") || document.querySelector("#repository-container-header");
  if (!toolbar) {
    return;
  }

  const button = document.createElement("button");
  button.id = BUTTON_ID;
  button.type = "button";
  button.className = "ddockit-button";
  button.textContent = "Run with TryThisSoftware";
  button.addEventListener("click", async () => {
    const payload = window.__ddockitRepositoryContext;
    if (!payload?.owner || !payload?.repo) {
      console.warn("TryThisSoftware repository context unavailable on this page.");
      return;
    }
    await chrome.runtime.sendMessage({ type: "DDOCKIT_OPEN_SIDEPANEL" });
    await chrome.runtime.sendMessage({ type: "DDOCKIT_DETECTED_REPOSITORY", payload });
  });

  toolbar.appendChild(button);
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", ensureButton, { once: true });
} else {
  ensureButton();
}
