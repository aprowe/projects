const SYSTEM_PROMPT = `You are a text rewriting service. The user will give you:
1. A rewriting instruction (e.g. "rewrite as if written by a child").
2. A JSON object whose keys are integers and values are strings of text extracted from a webpage.

Return a JSON object with the SAME keys, where each value is the corresponding text rewritten according to the instruction.

Rules:
- Preserve approximate length and meaning unless the instruction explicitly says otherwise.
- Keep proper nouns, numbers, and dates intact.
- Preserve any leading or trailing whitespace from the input string.
- Each output value must be a single string (no nesting, no arrays).
- Output ONLY the JSON object. No preamble, no commentary, no markdown fences.
- Every input key must appear in the output.`;

chrome.runtime.onMessage.addListener((msg, sender, sendResponse) => {
  if (msg.type === "rewrite") {
    handleRewrite(msg.textMap).then(sendResponse);
    return true;
  }
});

async function cacheKey(prompt, model, text) {
  const data = new TextEncoder().encode(`${prompt}\x00${model}\x00${text}`);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return (
    "c1_" +
    Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0"))
      .join("")
      .slice(0, 32)
  );
}

async function handleRewrite(textMap) {
  try {
    const { apiKey, prompt, model } = await chrome.storage.local.get([
      "apiKey",
      "prompt",
      "model",
    ]);
    if (!apiKey) {
      return {
        ok: false,
        error: "No API key set. Click the extension icon and add one.",
      };
    }
    if (!prompt) {
      return { ok: false, error: "No rewrite prompt set." };
    }
    const modelId = model || "claude-opus-4-7";

    const keyByIndex = {};
    for (const [i, text] of Object.entries(textMap)) {
      keyByIndex[i] = await cacheKey(prompt, modelId, text);
    }
    const cached = await chrome.storage.local.get(Object.values(keyByIndex));

    const result = {};
    const missMap = {};
    for (const [i, text] of Object.entries(textMap)) {
      const hit = cached[keyByIndex[i]];
      if (typeof hit === "string") result[i] = hit;
      else missMap[i] = text;
    }

    if (Object.keys(missMap).length === 0) {
      return { ok: true, result };
    }

    const resp = await fetch("https://api.anthropic.com/v1/messages", {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-api-key": apiKey,
        "anthropic-version": "2023-06-01",
        "anthropic-dangerous-direct-browser-access": "true",
      },
      body: JSON.stringify({
        model: modelId,
        max_tokens: 16000,
        system: SYSTEM_PROMPT,
        messages: [
          {
            role: "user",
            content: `Instruction: ${prompt}\n\nTexts to rewrite:\n${JSON.stringify(missMap)}`,
          },
        ],
      }),
    });

    if (!resp.ok) {
      const errText = await resp.text();
      return { ok: false, error: `API ${resp.status}: ${errText}` };
    }

    const data = await resp.json();
    const textBlock = data.content.find((b) => b.type === "text");
    if (!textBlock) {
      return { ok: false, error: "No text block in API response." };
    }

    const cleaned = textBlock.text
      .trim()
      .replace(/^```(?:json)?\s*/, "")
      .replace(/\s*```$/, "");

    let parsed;
    try {
      parsed = JSON.parse(cleaned);
    } catch (e) {
      return {
        ok: false,
        error: `Could not parse model output as JSON: ${e.message}`,
      };
    }

    const toCache = {};
    for (const [i, rewritten] of Object.entries(parsed)) {
      if (typeof rewritten !== "string") continue;
      if (missMap[i] === undefined) continue;
      result[i] = rewritten;
      toCache[keyByIndex[i]] = rewritten;
    }
    if (Object.keys(toCache).length) {
      chrome.storage.local.set(toCache).catch((e) => {
        console.warn("[Claude Rewriter] cache write failed:", e);
      });
    }

    return { ok: true, result };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}
