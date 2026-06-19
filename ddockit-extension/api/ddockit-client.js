const API_BASE_URL = "https://api.trythissoftware.com";

export async function launchExecution(payload) {
  const response = await fetch(`${API_BASE_URL}/api/v1/executions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json"
    },
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    throw new Error(`TryThisSoftware API error: ${response.status}`);
  }

  return response.json();
}
