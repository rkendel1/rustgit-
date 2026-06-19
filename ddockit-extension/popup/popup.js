const authStatus = document.getElementById("auth-status");
const deaStatus = document.getElementById("dea-status");
const openPortalButton = document.getElementById("open-portal");

chrome.storage.local.get(["ddockitAuth", "ddockitDeaConnected"], (state) => {
  authStatus.textContent = `Auth: ${state.ddockitAuth ? "signed in" : "not signed in"}`;
  deaStatus.textContent = `DEA: ${state.ddockitDeaConnected ? "connected" : "offline"}`;
});

openPortalButton.addEventListener("click", () => {
  chrome.tabs.create({ url: "https://trythissoftware.com" });
});
