const apiKey = document.getElementById("apiKey");
const prompt = document.getElementById("prompt");
const model = document.getElementById("model");
const run = document.getElementById("run");

(async () => {
  const stored = await chrome.storage.local.get(["apiKey", "prompt", "model"]);
  if (stored.apiKey) apiKey.value = stored.apiKey;
  prompt.value = stored.prompt || "rewrite as if written by a child";
  model.value = stored.model || "claude-opus-4-7";
})();

async function save() {
  await chrome.storage.local.set({
    apiKey: apiKey.value.trim(),
    prompt: prompt.value.trim(),
    model: model.value,
  });
}

[apiKey, prompt, model].forEach((el) => el.addEventListener("change", save));

document.querySelectorAll(".examples a").forEach((a) => {
  a.addEventListener("click", () => {
    prompt.value = a.dataset.preset;
    save();
  });
});

run.addEventListener("click", async () => {
  await save();
  if (!apiKey.value.trim()) {
    alert("Add your Anthropic API key first.");
    return;
  }
  if (!prompt.value.trim()) {
    alert("Add a rewrite prompt.");
    return;
  }
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !tab.id) return;
  if (
    tab.url &&
    (tab.url.startsWith("chrome://") ||
      tab.url.startsWith("chrome-extension://") ||
      tab.url.startsWith("edge://") ||
      tab.url.startsWith("about:"))
  ) {
    alert("Cannot run on browser-internal pages. Open a normal website first.");
    return;
  }
  try {
    await chrome.scripting.executeScript({
      target: { tabId: tab.id },
      files: ["content.js"],
    });
    window.close();
  } catch (e) {
    alert("Could not inject script: " + e.message);
  }
});
