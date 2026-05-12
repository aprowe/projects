(async function () {
  if (window.__claudeRewriterRunning) return;
  window.__claudeRewriterRunning = true;

  const banner = document.createElement("div");
  banner.id = "__claude-rewriter-banner";
  banner.style.cssText =
    "position:fixed;top:0;left:0;right:0;background:#1a1a1a;color:#fff;padding:10px 16px;font:14px system-ui,sans-serif;z-index:2147483647;border-bottom:2px solid #f80;box-shadow:0 2px 8px rgba(0,0,0,.3);display:flex;align-items:center;gap:12px";
  banner.innerHTML =
    '<span>🔁 Claude is rewriting this page…</span><span id="__claude-progress" style="opacity:.8">preparing…</span>';
  document.documentElement.appendChild(banner);

  const SKIP_TAGS = new Set([
    "SCRIPT",
    "STYLE",
    "NOSCRIPT",
    "TEMPLATE",
    "SVG",
    "MATH",
    "CODE",
    "PRE",
    "TEXTAREA",
    "INPUT",
    "KBD",
    "SAMP",
    "VAR",
  ]);

  function shouldSkip(node) {
    let p = node.parentElement;
    while (p) {
      if (SKIP_TAGS.has(p.nodeName)) return true;
      if (p.isContentEditable) return true;
      p = p.parentElement;
    }
    return false;
  }

  const walker = document.createTreeWalker(
    document.body,
    NodeFilter.SHOW_TEXT,
    {
      acceptNode(node) {
        if (!node.nodeValue || !node.nodeValue.trim()) {
          return NodeFilter.FILTER_REJECT;
        }
        if (shouldSkip(node)) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      },
    },
  );

  const nodes = [];
  let n;
  while ((n = walker.nextNode())) nodes.push(n);

  if (nodes.length === 0) {
    finish("Nothing to rewrite on this page.");
    return;
  }

  const MAX_CHARS = 3500;
  const groups = [];
  let group = [];
  let groupChars = 0;
  for (const node of nodes) {
    const len = node.nodeValue.length;
    if (groupChars + len > MAX_CHARS && group.length) {
      groups.push(group);
      group = [];
      groupChars = 0;
    }
    group.push(node);
    groupChars += len;
  }
  if (group.length) groups.push(group);

  const progress = banner.querySelector("#__claude-progress");
  let done = 0;
  let errored = 0;
  progress.textContent = `0 / ${groups.length} chunks`;

  const CONCURRENCY = 4;
  let cursor = 0;
  async function worker() {
    while (cursor < groups.length) {
      const idx = cursor++;
      const g = groups[idx];
      const textMap = {};
      g.forEach((node, i) => {
        textMap[i] = node.nodeValue;
      });
      try {
        const reply = await chrome.runtime.sendMessage({
          type: "rewrite",
          textMap,
        });
        if (reply && reply.ok && reply.result) {
          g.forEach((node, i) => {
            const v = reply.result[i];
            if (typeof v === "string") node.nodeValue = v;
          });
        } else {
          errored++;
          console.error("[Claude Rewriter]", reply && reply.error);
        }
      } catch (e) {
        errored++;
        console.error("[Claude Rewriter]", e);
      }
      done++;
      progress.textContent = `${done} / ${groups.length} chunks${errored ? ` (${errored} failed)` : ""}`;
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(CONCURRENCY, groups.length) }, worker),
  );

  finish(
    errored
      ? `Done with ${errored}/${groups.length} chunk(s) failed — see console.`
      : `Rewrote ${groups.length} chunk(s).`,
  );

  function finish(message) {
    banner.innerHTML = `<span>✅ ${message}</span><button id="__claude-close" style="margin-left:auto;background:#444;color:#fff;border:none;border-radius:4px;padding:5px 11px;cursor:pointer;font:inherit">close</button>`;
    banner.querySelector("#__claude-close").onclick = () => banner.remove();
    setTimeout(() => banner.remove(), 6000);
    window.__claudeRewriterRunning = false;
  }
})();
