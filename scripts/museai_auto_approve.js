// ==UserScript==
// @name         Muse AI Persistent Auto-Approve (v5.0.0)
// @namespace    https://muse.ai/
// @version      5.0.0
// @description  Survives page reloads via Tampermonkey/Violentmonkey or self-bootstrapping SessionStorage injector
// @match        https://muse.ai/*
// @match        https://*.muse.ai/*
// @run-at       document-idle
// @grant        none
// ==/UserScript==

(() => {
  "use strict";

  try {
    sessionStorage.setItem("__MUSE_AUTO_APPROVE_ENABLED", "1");
  } catch {}

  if (window.__museAutoApprove?.running) {
    console.info("[muse-auto-approve] already active");
    return window.__museAutoApprove;
  }

  const CFG = {
    intervalMs: 400,
    verbose: true,
  };

  let ticks = 0;
  let clicks = 0;
  const clickedElements = new WeakSet();

  function triggerClick(el, label) {
    if (!el || clickedElements.has(el)) return false;
    clickedElements.add(el);

    try {
      el.scrollIntoView({ block: "center", inline: "nearest" });
      el.focus?.({ preventScroll: true });
    } catch {}

    const opts = { bubbles: true, cancelable: true, view: window };
    ["pointerdown", "mousedown", "pointerup", "mouseup", "click"].forEach((type) => {
      try {
        el.dispatchEvent(new MouseEvent(type, opts));
      } catch {}
    });

    try {
      if (typeof el.click === "function") el.click();
    } catch {}

    clicks++;
    if (CFG.verbose) console.log(`✅ [muse-auto-approve] #${clicks}: ${label}`);
    setTimeout(() => clickedElements.delete(el), 3500);
    return true;
  }

  function isButtonInteractive(b) {
    return Boolean(b && !b.disabled && b.getAttribute("aria-disabled") !== "true");
  }

  function findPrimaryAction() {
    const primary = document.querySelector('button[data-hatch-approval-primary-action="true"]');
    if (isButtonInteractive(primary)) {
      return { el: primary, label: 'Primary "Allow" button' };
    }
    return null;
  }

  function findTextAllowAction() {
    const textAllow = Array.from(document.querySelectorAll("button")).find((b) => {
      if (!isButtonInteractive(b)) return false;
      const t = (b.innerText || b.textContent || "").trim().toLowerCase();
      return t === "allow" || t === "approve" || t.includes("allow once");
    });
    if (textAllow) {
      return { el: textAllow, label: 'Text "Allow" button' };
    }
    return null;
  }

  function findCardAction() {
    const closedCard = document.querySelector(
      '[data-slot="collapsible"][data-state="closed"] button[data-slot="collapsible-trigger"], button[data-slot="collapsible-trigger"][aria-expanded="false"]'
    );
    if (closedCard) {
      return { el: closedCard, label: "Expanded collapsed review card" };
    }
    return null;
  }

  function findApprovalsAction() {
    const approvalsTab = document.querySelector('button[aria-label="Approvals"]');
    if (approvalsTab && approvalsTab.getAttribute("aria-pressed") !== "true") {
      return { el: approvalsTab, label: "Switched to Approvals tab" };
    }
    return null;
  }

  function findAvatarAction() {
    const avatar = document.querySelector(
      '[data-hatch-avatar-interaction="true"], div[role="button"][aria-label="Muse"], button[aria-label="Muse"]'
    );
    if (avatar && !document.body.textContent.includes("Approvals")) {
      return { el: avatar, label: "Opened Muse avatar panel" };
    }
    return null;
  }

  const ACTION_FINDERS = [
    findPrimaryAction,
    findTextAllowAction,
    findCardAction,
    findApprovalsAction,
    findAvatarAction,
  ];

  function runWorkflow() {
    ticks++;
    for (const find of ACTION_FINDERS) {
      const action = find();
      if (action && triggerClick(action.el, action.label)) {
        return;
      }
    }
  }

  const timer = setInterval(runWorkflow, CFG.intervalMs);
  const observer = new MutationObserver(() => runWorkflow());
  if (document.documentElement) {
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
    });
  }

  runWorkflow();

  console.info("🚀 [muse-auto-approve] v5.0.0 active! Persistent across reloads.");

  const api = {
    running: true,
    stop() {
      clearInterval(timer);
      observer.disconnect();
      api.running = false;
      try {
        sessionStorage.removeItem("__MUSE_AUTO_APPROVE_ENABLED");
      } catch {}
      console.log("⏹️ Auto-approve stopped.");
    },
    stats: () => ({ ticks, clicks }),
    config: CFG,
  };

  window.__museAutoApprove = api;
  return api;
})();
