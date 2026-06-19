const API_BASE_URL = "https://api.trythissoftware.com";

chrome.runtime.onInstalled.addListener(() => {
  console.log("TryThisSoftware extension installed");
});

chrome.action.onClicked.addListener(async (tab) => {
  if (!tab?.id) {
    return;
  }

  try {
    await chrome.sidePanel.open({ tabId: tab.id });
  } catch (error) {
    console.error("Unable to open side panel", error);
  }
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (!message || typeof message !== "object") {
    return false;
  }

  if (message.type === "DDOCKIT_OPEN_SIDEPANEL") {
    if (sender.tab?.id) {
      chrome.sidePanel.open({ tabId: sender.tab.id }).catch((error) => {
        console.error("Unable to open side panel", error);
      });
    }
    sendResponse({ ok: true });
    return true;
  }

  if (message.type === "DDOCKIT_LAUNCH_REPO") {
    launchRepository(message.payload)
      .then((result) => sendResponse({ ok: true, result }))
      .catch((error) => sendResponse({ ok: false, error: String(error) }));
    return true;
  }

  if (message.type === "DDOCKIT_DETECTED_REPOSITORY") {
    chrome.storage.session.set({ detectedRepository: message.payload }).catch((error) => {
      console.error("Unable to persist repository context", error);
    });
    sendResponse({ ok: true });
    return true;
  }

  return false;
});

async function launchRepository(payload) {
  if (!payload?.owner || !payload?.repo) {
    throw new Error("Missing repository payload");
  }

  const response = await fetch(`${API_BASE_URL}/api/v1/executions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify({
      owner: payload.owner,
      repo: payload.repo,
      branch: payload.branch || "main"
    })
  });

  if (!response.ok) {
    throw new Error(`TryThisSoftware launch failed (${response.status})`);
  }

  const data = await response.json();

  if (data.workspace_url) {
    await chrome.notifications.create({
      type: "basic",
      iconUrl: "assets/icon128.png",
      title: "TryThisSoftware run started",
      message: `Workspace ready: ${data.workspace_url}`
    });
  }

  return data;
}
