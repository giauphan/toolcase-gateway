(function() {
  "use strict";

  function byId(id) {
    return document.getElementById(id);
  }

  function escapeHtml(str) {
    if (!str) return "";
    return String(str)
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;")
      .replaceAll("'", "&#39;");
  }

  function parseJsonSafely(text) {
    if (!text) return {};
    try {
      return JSON.parse(text);
    } catch (_) {
      return {};
    }
  }

  function postJson(url, payload) {
    return fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload)
    }).then(function(res) {
      return res.text().then(function(text) {
        return { ok: res.ok, status: res.status, json: parseJsonSafely(text) };
      });
    });
  }

  function setMessageVisible(element, visible) {
    if (element) {
      element.classList.toggle("is-visible", visible);
      element.classList.toggle("is-hidden", !visible);
    }
  }

  let toastTimer = 0;
  function showToast(message, isError) {
    const el = byId("toast");
    if (!el) return;
    el.textContent = message;
    el.classList.toggle("toast-err", Boolean(isError));
    setMessageVisible(el, true);
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function() {
      setMessageVisible(el, false);
    }, 3400);
  }

  // DOM Elements
  const omniTarget = byId("omni-target");
  const omniDesc = byId("omni-desc");
  const jevStatus = byId("jev-status");
  const jevDesc = byId("jev-desc");
  const openrouterStatus = byId("openrouter-status");
  const openrouterDesc = byId("openrouter-desc");
  const museStatus = byId("muse-status");
  const museDesc = byId("muse-desc");

  // Gateway Config Form
  const cfgForm = byId("config-form");
  const cfgHost = byId("cfg-host");
  const cfgPort = byId("cfg-port");
  const cfgDefaultModel = byId("cfg-default-model");
  const cfgTimeout = byId("cfg-timeout");
  const cfgFallbacks = byId("cfg-fallbacks");
  const saveCfgBtn = byId("save-cfg-btn");
  const cfgOkMsg = byId("cfg-ok-msg");
  const cfgErrMsg = byId("cfg-err-msg");

  // Muse Config Form
  const museForm = byId("muse-form");
  const cfgMuseBase = byId("cfg-muse-base");
  const cfgMuseWs = byId("cfg-muse-ws");
  const cfgMuseVmid = byId("cfg-muse-vmid");
  const cfgMuseToken = byId("cfg-muse-token");
  const cfgMuseNotary = byId("cfg-muse-notary");
  const cfgMuseCookie = byId("cfg-muse-cookie");
  const hintToken = byId("hint-token");
  const hintNotary = byId("hint-notary");
  const hintCookie = byId("hint-cookie");
  const cfgMuseVideoPrompt = byId("cfg-muse-video-prompt");
  const cfgMuseApprovalMode = byId("cfg-muse-approval-mode");
  const cfgMuseApprovalDestinations = byId("cfg-muse-approval-destinations");
  const cfgMuseApprovalTypes = byId("cfg-muse-approval-types");
  const cfgMuseApprovalTimeout = byId("cfg-muse-approval-timeout");
  const saveMuseBtn = byId("save-muse-btn");
  const museOkMsg = byId("muse-ok-msg");
  const museErrMsg = byId("muse-err-msg");

  // Accounts Form & Table
  const tableBody = byId("accounts-tbody");
  const countBadge = byId("accounts-count-badge");
  const addForm = byId("accounts-form");
  const keysInput = byId("jev-keys-input");
  const labelInput = byId("jev-label-input");
  const providerSelect = byId("provider-select");
  const submitBtn = byId("add-accounts-btn");
  const okMsg = byId("accounts-ok-msg");
  const errMsg = byId("accounts-err-msg");

  // Jev Config Form
  const jevConfigForm = byId("jev-config-form");
  const cfgJevBase = byId("cfg-jev-base");
  const cfgJevOrBase = byId("cfg-jev-or-base");
  const cfgJevOrModel = byId("cfg-jev-or-model");
  const cfgJevOrFallbackModels = byId("cfg-jev-or-fallbacks");
  const cfgJevOrEnabled = byId("cfg-jev-or-enabled");
  const cfgJevCooldown = byId("cfg-jev-cooldown");
  const saveJevBtn = byId("save-jev-btn");
  const jevOkMsg = byId("jev-ok-msg");
  const jevErrMsg = byId("jev-err-msg");

  // Modules Section
  const modulesTbody = byId("modules-tbody");
  const modulesCardsWrap = byId("modules-cards-wrap");
  const modulesTableWrap = byId("modules-table-wrap");
  const modulesSearch = byId("modules-search");
  const modulesStatusFilter = byId("modules-status-filter");
  const modulesKindFilter = byId("modules-kind-filter");
  const modulesFeedback = byId("modules-feedback");
  const moduleActionDialog = byId("module-action-dialog");
  const moduleActionDescription = byId("module-action-description");
  const moduleActionConfirm = byId("module-action-confirm");
  const moduleDetailDialog = byId("module-detail-dialog");
  const moduleDetailContent = byId("module-detail-content");
  const modulesCountBadge = byId("modules-count-badge");
  const listModeBtn = byId("listMode");
  const cardModeBtn = byId("cardMode");

  let allModules = [];
  let renderedModules = [];
  let pendingModuleAction = null;
  const moduleRequestsInFlight = new Set();

  // Catalog Section
  const catalogGrid = byId("catalog-grid");
  const customInstallDetails = byId("custom-install-details");
  const customInstallBtn = byId("customInstall");
  const installForm = byId("install-module-form");
  const modSource = byId("mod-source");
  const modZipFile = byId("mod-zip-file");
  const modId = byId("mod-id");
  const modName = byId("mod-name");
  const modNamespace = byId("mod-namespace");
  const modEntrypoint = byId("mod-entrypoint");
  const modCaps = byId("mod-caps");
  const installBtn = byId("install-module-btn");
  const modOkMsg = byId("mod-ok-msg");
  const modErrMsg = byId("mod-err-msg");

  // Navigation and Views
  const views = {
    overview: byId("view-overview"),
    modules: byId("view-modules"),
    catalog: byId("view-catalog"),
    providers: byId("view-providers"),
    accounts: byId("view-accounts"),
    mcp: byId("view-mcp"),
    studios: byId("view-studios"),
    settings: byId("view-settings"),
  };

  const breadcrumbMap = {
    overview: "Gateway / Overview",
    modules: "Gateway / Apps / Installed Modules",
    catalog: "Gateway / Apps / Module Catalog",
    providers: "Gateway / Providers",
    accounts: "Gateway / Accounts",
    mcp: "Gateway / MCP",
    studios: "Gateway / Studios",
    settings: "Gateway / Settings",
  };

  const ROUTE_ALIASES = {
    overview: "overview",
    modules: "modules",
    "modules-section": "modules",
    catalog: "catalog",
    "module-catalog": "catalog",
    providers: "providers",
    "provider-settings": "providers",
    "muse-settings": "providers",
    accounts: "accounts",
    "accounts-settings": "accounts",
    mcp: "mcp",
    "mcp-section": "mcp",
    studios: "studios",
    "studios-section": "studios",
    settings: "settings",
    "settings-section": "settings"
  };

  function normalizeRoute(hash) {
    const raw = (hash || "").replace(/^#/, "").trim().toLowerCase();
    return ROUTE_ALIASES[raw] || "overview";
  }

  function navigateTo(viewKey, tabKey) {
    const key = normalizeRoute(viewKey);
    Object.keys(views).forEach(function(k) {
      if (views[k]) {
        const isTarget = k === key;
        views[k].classList.toggle("visible", isTarget);
      }
    });

    const navItems = document.querySelectorAll(".navitem, .ui-nav-link");
    navItems.forEach(function(item) {
      const active = item.dataset.go === key;
      item.classList.toggle("active", active);
      if (active) item.setAttribute("aria-current", "page");
      else item.removeAttribute("aria-current");
    });

    const breadcrumb = byId("dashboard-breadcrumb");
    if (breadcrumb) {
      const label = breadcrumbMap[key] || "Gateway / Overview";
      const parts = label.split(" / ");
      const last = parts.pop();
      breadcrumb.innerHTML = escapeHtml(parts.join(" / ")) + " / <span class=\"breadactive\">" + escapeHtml(last) + "</span>";
    }

    const sidebar = byId("sidebar");
    const menubtn = byId("menubtn");
    if (sidebar) sidebar.classList.remove("open");
    if (menubtn) menubtn.setAttribute("aria-expanded", "false");

    if (key === "providers" && tabKey) {
      switchProviderTab(tabKey);
    }

    window.scrollTo(0, 0);
  }

  function switchProviderTab(providerId) {
    const tabs = document.querySelectorAll(".tabline .tab");
    tabs.forEach(function(tab) {
      const active = tab.dataset.provider === providerId;
      tab.classList.toggle("active", active);
      tab.setAttribute("aria-selected", active ? "true" : "false");
    });

    document.querySelectorAll(".provider-panel").forEach(function(panel) {
      const isCurrent = panel.id === ("panel-" + providerId);
      panel.classList.toggle("is-hidden", !isCurrent);
    });
  }

  // Module view mode toggling (List vs Cards)
  function setModuleViewMode(mode) {
    if (listModeBtn) {
      listModeBtn.classList.toggle("selected", mode === "list");
      listModeBtn.setAttribute("aria-pressed", mode === "list" ? "true" : "false");
    }
    if (cardModeBtn) {
      cardModeBtn.classList.toggle("selected", mode === "card");
      cardModeBtn.setAttribute("aria-pressed", mode === "card" ? "true" : "false");
    }
    if (modulesTableWrap) modulesTableWrap.hidden = mode !== "list";
    if (modulesCardsWrap) modulesCardsWrap.hidden = mode !== "card";
    filterModules();
  }

  function initToolbarAndNavigation() {
    if (listModeBtn) listModeBtn.onclick = function() { setModuleViewMode("list"); };
    if (cardModeBtn) cardModeBtn.onclick = function() { setModuleViewMode("card"); };

    // Provider tabs wiring
    document.querySelectorAll(".tabline .tab").forEach(function(btn) {
      btn.addEventListener("click", function() {
        if (btn.dataset.provider) {
          switchProviderTab(btn.dataset.provider);
        }
      });
    });

    // Theme toggle
    const themebtn = byId("themebtn");
    if (themebtn) {
      themebtn.onclick = function() {
        document.body.classList.toggle("dark");
        const isDark = document.body.classList.contains("dark");
        try {
          localStorage.setItem("tg_theme", isDark ? "dark" : "light");
        } catch (_) {}
      };
    }

    // Restore saved theme
    try {
      if (localStorage.getItem("tg_theme") === "dark") {
        document.body.classList.add("dark");
      }
    } catch (_) {}

    // Mobile drawer button
    const menubtn = byId("menubtn");
    if (menubtn) {
      menubtn.onclick = function() {
        const sidebar = byId("sidebar");
        if (sidebar) {
          sidebar.classList.toggle("open");
          const isOpen = sidebar.classList.contains("open");
          menubtn.setAttribute("aria-expanded", isOpen ? "true" : "false");
        }
      };
    }

    // Apps button
    const appsbtn = byId("appsbtn");
    if (appsbtn) {
      appsbtn.onclick = function() {
        window.location.hash = "catalog";
        navigateTo("catalog");
      };
    }

    // Custom module button in catalog
    if (customInstallBtn && customInstallDetails) {
      customInstallBtn.onclick = function() {
        customInstallDetails.open = true;
        customInstallDetails.scrollIntoView({ behavior: "smooth" });
      };
    }

    // Top Search with ⌘ K shortcut
    const topsearch = byId("topsearch");
    if (topsearch) {
      topsearch.addEventListener("keydown", function(e) {
        if (e.key === "Enter") {
          const val = topsearch.value.trim();
          if (modulesSearch) modulesSearch.value = val;
          window.location.hash = "modules";
          navigateTo("modules");
          filterModules();
        }
      });
    }
  }

  document.addEventListener("keydown", function(e) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (topsearch) topsearch.focus();
    }
    if (e.key === "Escape") {
      const sidebar = byId("sidebar");
      if (sidebar) sidebar.classList.remove("open");
      if (menubtn) menubtn.setAttribute("aria-expanded", "false");
    }
  });

  // Global link handling for data-go navigation
  document.addEventListener("click", function(e) {
    const goEl = e.target.closest("[data-go]");
    if (goEl) {
      const target = goEl.dataset.go;
      const tab = goEl.dataset.tab;
      window.location.hash = target;
      navigateTo(target, tab);
      return;
    }

    const toggleBtn = e.target.closest(".module-toggle");
    if (toggleBtn) {
      requestModuleStateChange(toggleBtn.dataset.moduleId, toggleBtn.dataset.enable === "true");
      return;
    }

    const remBtn = e.target.closest(".module-remove");
    if (remBtn) {
      requestModuleRemoval(remBtn.dataset.moduleId);
      return;
    }

    const detailBtn = e.target.closest(".module-detail");
    if (detailBtn) {
      showModuleDetail(detailBtn.dataset.moduleId);
      return;
    }

    const tabBtn = e.target.closest(".tabline .tab");
    if (tabBtn?.dataset?.provider) {
      switchProviderTab(tabBtn.dataset.provider);
      return;
    }

    const moveBtn = e.target.closest(".module-move");
    if (moveBtn?.dataset?.moduleId && moveBtn?.dataset?.direction) {
      window.updateModulePosition(moveBtn.dataset.moduleId, { direction: moveBtn.dataset.direction });
    }
  });

  document.addEventListener("change", function(e) {
    const posSelect = e.target.closest(".module-pos-select");
    if (posSelect?.dataset?.moduleId) {
      window.updateModulePosition(posSelect.dataset.moduleId, { menu_position: posSelect.value });
      return;
    }

    const harInput = e.target.closest(".dynamic-har-file");
    if (harInput?.files?.[0] && harInput.dataset.harEndpoint) {
      const file = harInput.files[0];
      const modId = harInput.dataset.moduleId || "module";
      file.text().then(function(rawText) {
        return fetch(harInput.dataset.harEndpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: rawText
        });
      })
      .then(function(r) { return r.json().then(function(j) { return { ok: r.ok, json: j }; }); })
      .then(function(res) {
        if (!res.ok) {
          showToast(res.json.error?.message || res.json.message || "HAR import failed", true);
        } else {
          showToast(res.json.message || ("Imported HAR for " + modId), false);
          loadOverview();
        }
      })
      .catch(function(err) {
        showToast("HAR import failed: " + err.message, true);
      })
      .finally(function() {
        harInput.value = "";
      });
    }
  });

  // Module filter inputs
  [modulesSearch, modulesStatusFilter, modulesKindFilter].forEach(function(control) {
    if (control) {
      control.addEventListener("input", filterModules);
      control.addEventListener("change", filterModules);
    }
  });

  function initModuleActionDialog() {
    if (!moduleActionDialog) return;
    moduleActionDialog.addEventListener("close", function() {
      const action = pendingModuleAction;
      pendingModuleAction = null;
      if (moduleActionDialog.returnValue !== "confirm" || !action) return;
      if (action.kind === "remove") window.removeModule(action.id);
      else window.toggleModule(action.id, action.enable);
    });
  }

  function showModulesFeedback(message, isError) {
    if (!modulesFeedback) return;
    modulesFeedback.textContent = message;
    modulesFeedback.dataset.state = isError ? "error" : "success";
    showToast(message, isError);
  }

  function filterModules() {
    const query = modulesSearch ? modulesSearch.value.trim().toLowerCase() : "";
    const status = modulesStatusFilter ? modulesStatusFilter.value : "all";
    const kind = modulesKindFilter ? modulesKindFilter.value : "all";
    const filtered = allModules.filter(function(module) {
      const searchable = [module.id, module.name, module.namespace]
        .concat(module.capabilities || [])
        .join(" ")
        .toLowerCase();
      return (!query || searchable.includes(query))
        && (status === "all" || module.status === status)
        && (kind === "all" || module.kind === kind);
    });
    renderModules(filtered, true);
  }

  function requestModuleStateChange(id, enable) {
    if (!id || !moduleActionDialog) return;
    pendingModuleAction = { id: id, enable: enable, kind: 'state' };
    moduleActionDescription.textContent = enable
      ? 'Enable this module? Its registered routes will accept requests again.'
      : 'Disable this module? Its registered routes will fail closed with HTTP 503 until it is re-enabled.';
    moduleActionConfirm.textContent = enable ? 'Enable module' : 'Disable module';
    moduleActionDialog.returnValue = '';
    moduleActionDialog.showModal();
  }

  function requestModuleRemoval(id) {
    if (!id || !moduleActionDialog) return;
    pendingModuleAction = { id: id, kind: 'remove' };
    moduleActionDescription.textContent = 'Remove this personal module? This removes its installed files and cannot be undone.';
    moduleActionConfirm.textContent = 'Remove module';
    moduleActionDialog.returnValue = '';
    moduleActionDialog.showModal();
  }

  function showModuleDetail(id) {
    const module = renderedModules.find(function(candidate) { return candidate.id === id; });
    if (!module || !moduleDetailDialog || !moduleDetailContent) return;

    const capabilities = module.capabilities?.length
      ? module.capabilities.map(function(c) {
          return "<span class=\"cap\">" + escapeHtml(c) + "</span>";
        }).join(" ")
      : "<span class=\"hint\">None declared</span>";

    const providers = module.providers?.length
      ? module.providers.map(function(p) {
          const subRoute = p.prefix || (module.namespace.replace(/\/$/, "") + (p.subpath || ""));
          return "<li><strong>" + escapeHtml(p.id) + "</strong> (" + escapeHtml(p.name || p.kind || "provider") + ") · " +
            "<code>" + escapeHtml(subRoute) + "</code> · " +
            "<span class=\"pill " + (p.status === "enabled" ? "good" : "gray") + "\">" + escapeHtml(p.status) + "</span></li>";
        }).join("")
      : "<li>No nested sub-providers</li>";

    const tabTarget = module.settings_tab || (module.id === "omniroute" ? "omni" : module.id);
    const cfgUrl = module.config_url || ("/modules/" + module.id + "/config");

    moduleDetailContent.innerHTML =
      "<div class=\"formgroup\">" +
        "<div><strong>Module ID:</strong> " + escapeHtml(module.id) + "</div>" +
        "<div><strong>Display Name:</strong> " + escapeHtml(module.name) + "</div>" +
        "<div><strong>Kind:</strong> <span class=\"pill gray\">" + escapeHtml(module.kind) + "</span></div>" +
        "<div><strong>Route Namespace:</strong> <code>" + escapeHtml(module.namespace) + "</code></div>" +
        "<div><strong>Status:</strong> <span class=\"pill " + (module.status === "enabled" ? "good" : "gray") + "\">" + escapeHtml(module.status) + "</span></div>" +
        "<div><strong>Capabilities:</strong><br>" + capabilities + "</div>" +
        "<div><strong>Nested Providers:</strong><ul>" + providers + "</ul></div>" +
        "<div class=\"btns\" style=\"margin-top:12px;\">" +
          "<a class=\"btn primary btn-primary\" data-go=\"providers\" data-tab=\"" + escapeHtml(tabTarget) + "\" href=\"#provider-settings\">Configure in Provider Settings</a> " +
          "<a class=\"btn\" href=\"" + escapeHtml(cfgUrl) + "\">Open Standalone UI Config ↗</a>" +
        "</div>" +
      "</div>";

    moduleDetailDialog.showModal();
  }

  function renderModuleActions(m, isBuiltin, isEnabled, isBusy) {
    const busyAttr = isBusy ? " disabled" : "";
    const toggleAction = isEnabled ? "false" : "true";
    const toggleLabel = isEnabled ? "Disable" : "Enable";
    const toggleBtn = "<button class=\"btn module-toggle\" data-module-id=\"" + escapeHtml(m.id) + "\" data-enable=\"" + toggleAction + "\"" + busyAttr + ">" + toggleLabel + "</button>";
    const tabTarget = m.settings_tab || (m.id === "omniroute" ? "omni" : m.id);
    const cfgUrl = m.config_url || ("/modules/" + m.id + "/config");
    const curPos = m.menu_position || "providers";
    const posSelect = " <select class=\"field module-pos-select\" data-module-id=\"" + escapeHtml(m.id) + "\" aria-label=\"Menu position for " + escapeHtml(m.name) + "\"" + busyAttr + ">" +
      "<option value=\"providers\"" + (curPos === "providers" ? " selected" : "") + ">Menu: Providers</option>" +
      "<option value=\"sidebar\"" + (curPos === "sidebar" ? " selected" : "") + ">Menu: Sidebar</option>" +
      "<option value=\"studios\"" + (curPos === "studios" ? " selected" : "") + ">Menu: Studios</option>" +
      "<option value=\"hidden\"" + (curPos === "hidden" ? " selected" : "") + ">Menu: Hidden</option>" +
    "</select>";
    const orderBtns = " <button class=\"btn module-move\" data-module-id=\"" + escapeHtml(m.id) + "\" data-direction=\"up\" title=\"Move up in menu\"" + busyAttr + ">↑</button>" +
      " <button class=\"btn module-move\" data-module-id=\"" + escapeHtml(m.id) + "\" data-direction=\"down\" title=\"Move down in menu\"" + busyAttr + ">↓</button>";
    const configBtns = " <a class=\"btn\" data-go=\"providers\" data-tab=\"" + escapeHtml(tabTarget) + "\" href=\"#provider-settings\">Settings</a>" +
      " <a class=\"btn\" href=\"" + escapeHtml(cfgUrl) + "\">UI Config ↗</a>";
    let removeBtn = "";
    if (!isBuiltin) {
      removeBtn = " <button class=\"btn module-remove\" data-module-id=\"" + escapeHtml(m.id) + "\"" + busyAttr + ">Remove</button>";
    }
    return toggleBtn + configBtns + posSelect + orderBtns + removeBtn;
  }

  function renderModules(modules, isFiltered) {
    renderedModules = modules;
    if (modulesCountBadge && !isFiltered) {
      const activeCount = modules.filter(function(m) { return m.status === "enabled"; }).length;
      modulesCountBadge.textContent = activeCount + " active";
    }

    if (!modules.length) {
      const emptyMsg = isFiltered ? "No modules match the current filters." : "No modules registered.";
      if (modulesTbody) {
        modulesTbody.innerHTML = "<tr><td colspan=\"7\" class=\"empty-hint\">" + emptyMsg + "</td></tr>";
      }
      if (modulesCardsWrap) {
        modulesCardsWrap.innerHTML = "<div class=\"empty-hint ui-grid-full\">" + emptyMsg + "</div>";
      }
      return;
    }

    // Render Table Mode
    if (modulesTbody) {
      modulesTbody.innerHTML = modules.map(function(m) {
        const isEnabled = m.status === "enabled";
        const isBuiltin = m.kind === "builtin";
        const isBusy = moduleRequestsInFlight.has(m.id);
        const caps = (m.capabilities || []).map(function(c) {
          return "<span class=\"cap\">" + escapeHtml(c) + "</span>";
        }).join("");

        const actionBtns = renderModuleActions(m, isBuiltin, isEnabled, isBusy);

        return "<tr>" +
          "<td><button class=\"name-link module-detail\" data-module-id=\"" + escapeHtml(m.id) + "\">" + escapeHtml(m.id) + "</button></td>" +
          "<td>" + escapeHtml(m.name) + "</td>" +
          "<td><span class=\"pill " + (isBuiltin ? "gray" : "personal") + "\">" + escapeHtml(m.kind) + "</span></td>" +
          "<td><code>" + escapeHtml(m.namespace) + "</code></td>" +
          "<td>" + (caps || "<span class=\"hint\">None</span>") + "</td>" +
          "<td><span class=\"pill " + (isEnabled ? "good" : "gray") + "\">" + (isEnabled ? "Enabled" : "Disabled") + "</span></td>" +
          "<td>" + actionBtns + "</td>" +
        "</tr>";
      }).join("");
    }

    // Render Card Mode
    if (modulesCardsWrap) {
      modulesCardsWrap.innerHTML = modules.map(function(m) {
        const isEnabled = m.status === "enabled";
        const isBuiltin = m.kind === "builtin";
        const isBusy = moduleRequestsInFlight.has(m.id);
        const caps = (m.capabilities || []).map(function(c) {
          return "<span class=\"cap\">" + escapeHtml(c) + "</span>";
        }).join(" ");

        const actionBtns = renderModuleActions(m, isBuiltin, isEnabled, isBusy);

        return "<div class=\"card catalogitem\">" +
          "<div class=\"catalogtitle\">" +
            "<button class=\"name-link module-detail\" data-module-id=\"" + escapeHtml(m.id) + "\">" + escapeHtml(m.name) + "</button> " +
            "<span class=\"pill " + (isBuiltin ? "gray" : "personal") + "\">" + escapeHtml(m.kind) + "</span>" +
          "</div>" +
          "<div class=\"catalogdesc\"><code>" + escapeHtml(m.namespace) + "</code><br>" + (caps || "None") + "</div>" +
          "<div class=\"minirow\">" +
            "<span class=\"pill " + (isEnabled ? "good" : "gray") + "\">" + (isEnabled ? "Enabled" : "Disabled") + "</span>" +
            actionBtns +
          "</div>" +
        "</div>";
      }).join("");
    }
  }

  function renderCatalog(catalog) {
    if (!catalogGrid) return;
    if (!catalog?.length) {
      catalogGrid.innerHTML = "<div class=\"empty-hint ui-grid-full\">No catalog entries declared.</div>";
      return;
    }

    catalogGrid.innerHTML = catalog.map(function(item) {
      const isInstalled = allModules.some(function(m) {
        return m.id === item.id
          || (m.namespace && item.namespace && m.namespace.toLowerCase() === item.namespace.toLowerCase())
          || (item.id === "tiktok-studio" && (m.namespace === "/tiktok" || m.id.toLowerCase().includes("tiktok")));
      });
      const caps = (item.capabilities || []).map(function(c) {
        return "<span class=\"cap\">" + escapeHtml(c) + "</span>";
      }).join(" ");

      const sourceHtml = item.source
        ? ("<br><span class=\"hint\">Repo: <code>" + escapeHtml(item.source) + "</code></span>")
        : "";

      return "<div class=\"card catalogitem\">" +
        "<span class=\"pill gray\">Catalog template</span>" +
        "<div class=\"catalogtitle\">" + escapeHtml(item.name) + "</div>" +
        "<div class=\"catalogdesc\">" + escapeHtml(item.description) + "<br><code>" + escapeHtml(item.namespace) + "</code>" + sourceHtml + "<br>" + caps + "</div>" +
        "<div class=\"minirow\">" +
          (isInstalled
            ? "<span class=\"pill good\">Installed</span>"
            : "<button class=\"btn catalog-install-btn\" data-catalog-id=\"" + escapeHtml(item.id) + "\">Explore &amp; install</button>") +
        "</div>" +
      "</div>";
    }).join("");
  }

  // Catalog grid install click handler
  function initCatalogGrid() {
    if (!catalogGrid) return;
    catalogGrid.addEventListener("click", function(ev) {
      const btn = ev.target.closest(".catalog-install-btn");
      if (!btn) return;
      const id = btn.dataset.catalogId;
      if (!id) return;
      btn.disabled = true;
      btn.textContent = "Installing...";
      executeModuleInstall({ id: id }, btn);
    });
  }

  function executeModuleInstall(payload, catalogButton) {
    postJson("/api/v1/dashboard/modules/install", payload)
      .then(function(result) {
        if (catalogButton) {
          catalogButton.disabled = false;
          catalogButton.textContent = "Explore & install";
        }
        if (!result.ok) {
          const errorDetail = result.json.message || ("HTTP " + result.status);
          showModulesFeedback('Failed to install module: ' + errorDetail, true);
        } else {
          showModulesFeedback("Module '" + (result.json.module ? result.json.module.id : "package") + "' installed successfully.", false);
          loadOverview();
        }
      })
      .catch(function(err) {
        if (catalogButton) {
          catalogButton.disabled = false;
          catalogButton.textContent = "Explore & install";
        }
        showModulesFeedback('Network error: ' + err.message, true);
      });
  }

  function sendModuleRequest(id, endpoint, payload, initialMsg, successMsg, failPrefix) {
    moduleRequestsInFlight.add(id);
    renderModules(renderedModules, true);
    if (initialMsg) {
      showModulesFeedback(initialMsg, false);
    }

    postJson(endpoint, payload)
      .then(function(result) {
        moduleRequestsInFlight.delete(id);
        if (!result.ok) {
          const errorDetail = result.json.message || ("HTTP " + result.status);
          showModulesFeedback(failPrefix + ": " + errorDetail, true);
          renderModules(renderedModules, true);
        } else {
          showModulesFeedback(successMsg, false);
          loadOverview();
        }
      })
      .catch(function(err) {
        moduleRequestsInFlight.delete(id);
        renderModules(renderedModules, true);
        showModulesFeedback("Network error: " + err.message, true);
      });
  }

  window.toggleModule = function(id, enable) {
    sendModuleRequest(
      id,
      "/api/v1/dashboard/modules/state",
      { id: id, enabled: enable },
      (enable ? "Enabling" : "Disabling") + " module '" + id + "'...",
      "Module '" + id + "' is now " + (enable ? "enabled" : "disabled") + ".",
      "Failed to update module state"
    );
  };

  window.updateModulePosition = function(id, opts) {
    sendModuleRequest(
      id,
      "/api/v1/dashboard/modules/position",
      { id: id, ...opts },
      null,
      "Module '" + id + "' menu position updated.",
      "Failed to update module menu position"
    );
  };

  function renderDynamicConfigField(f, m, cfg) {
    const key = f.key;
    const label = f.label || key;
    const type = f.type || "text";
    const isSecret = Boolean(f.secret || type === "password");
    const inputId = "dyn-cfg-" + m.id + "-" + key;
    if (isSecret) {
      const masked = cfg["masked_" + key] || "";
      return "<div class=\"field-group formgroup\">" +
        "<label for=\"" + escapeHtml(inputId) + "\">" + escapeHtml(label) + "</label>" +
        "<input type=\"password\" id=\"" + escapeHtml(inputId) + "\" data-key=\"" + escapeHtml(key) + "\" data-type=\"password\" autocomplete=\"off\" placeholder=\"Leave empty to keep existing\">" +
        "<div class=\"hint\">" + escapeHtml(masked ? ("Current: " + masked) : "Current: not set") + "</div>" +
      "</div>";
    }
    if (type === "boolean") {
      const boolVal = cfg[key] !== undefined ? Boolean(cfg[key]) : (f.default !== false);
      return "<div class=\"field-group formgroup\">" +
        "<label for=\"" + escapeHtml(inputId) + "\">" + escapeHtml(label) + "</label>" +
        "<select id=\"" + escapeHtml(inputId) + "\" data-key=\"" + escapeHtml(key) + "\" data-type=\"boolean\">" +
          "<option value=\"true\"" + (boolVal ? " selected" : "") + ">Enabled</option>" +
          "<option value=\"false\"" + (!boolVal ? " selected" : "") + ">Disabled</option>" +
        "</select>" +
      "</div>";
    }
    if (type === "number") {
      let numVal = 30;
      if (cfg[key] != null) {
        numVal = cfg[key];
      } else if (f.default != null) {
        numVal = f.default;
      }
      return "<div class=\"field-group formgroup\">" +
        "<label for=\"" + escapeHtml(inputId) + "\">" + escapeHtml(label) + "</label>" +
        "<input type=\"number\" id=\"" + escapeHtml(inputId) + "\" data-key=\"" + escapeHtml(key) + "\" data-type=\"number\" value=\"" + escapeHtml(numVal) + "\">" +
      "</div>";
    }
    const strVal = cfg[key] != null ? String(cfg[key]) : "";
    return "<div class=\"field-group formgroup\">" +
      "<label for=\"" + escapeHtml(inputId) + "\">" + escapeHtml(label) + "</label>" +
      "<input type=\"text\" id=\"" + escapeHtml(inputId) + "\" data-key=\"" + escapeHtml(key) + "\" data-type=\"text\" value=\"" + escapeHtml(strVal) + "\" placeholder=\"" + escapeHtml(f.placeholder || "") + "\">" +
    "</div>";
  }

  function resolveDynamicHarEndpoint(schema, m) {
    if (!schema.har_endpoint) {
      return "";
    }
    if (schema.har_endpoint.startsWith(m.namespace)) {
      return schema.har_endpoint;
    }
    return m.namespace.replace(/\/$/, "") + "/v1/config/har";
  }

  function renderDynamicProviderMenus(modules, moduleConfigs) {
    const visibleModules = (modules || []).filter(function(m) {
      return (m.menu_position || "providers") !== "hidden";
    });

    // 1. Overview Provider Modules list
    const overviewList = byId("overview-provider-list");
    if (overviewList && visibleModules.length) {
      overviewList.innerHTML = visibleModules.map(function(m) {
        const tabKey = m.settings_tab || (m.id === "omniroute" ? "omni" : m.id);
        const icon = m.icon || "◫";
        const kindLabel = m.kind === "builtin" ? "Built-in" : "Personal";
        const desc = m.description || ("Routed at " + m.namespace);
        return "<a class=\"provider-row\" data-go=\"providers\" href=\"#provider-settings\" data-tab=\"" + escapeHtml(tabKey) + "\">" +
          "<div class=\"provider-icon\" aria-hidden=\"true\">" + escapeHtml(icon) + "</div>" +
          "<div class=\"provider-main\">" +
            "<div class=\"provider-name\">" + escapeHtml(m.name) + " <span class=\"pill gray\">" + escapeHtml(kindLabel) + "</span></div>" +
            "<div class=\"provider-desc\">" + escapeHtml(desc) + "</div>" +
          "</div>" +
          "<div class=\"provider-cta\" aria-hidden=\"true\">›</div>" +
        "</a>";
      }).join("");
    }

    // 2. Sidebar Pinned Modules
    const sidebarPinned = byId("sidebar-pinned-modules");
    if (sidebarPinned) {
      const pinned = (modules || []).filter(function(m) {
        return m.menu_position === "sidebar";
      });
      if (!pinned.length) {
        sidebarPinned.innerHTML = "";
      } else {
        sidebarPinned.innerHTML = "<div class=\"sidebar-label ui-sidebar-label\">PINNED PROVIDERS</div>" +
          pinned.map(function(m) {
            const tabKey = m.settings_tab || (m.id === "omniroute" ? "omni" : m.id);
            const icon = m.icon || "◫";
            return "<a class=\"navitem ui-nav-link\" data-go=\"providers\" data-tab=\"" + escapeHtml(tabKey) + "\" href=\"#provider-settings\">" +
              "<span class=\"navicon\" aria-hidden=\"true\">" + escapeHtml(icon) + "</span>" +
              "<span>" + escapeHtml(m.name) + "</span>" +
            "</a>";
          }).join("");
      }
    }

    // 3. Provider Settings Tabs & Automatic Module UI Config Panels
    const tabline = byId("provider-tabline");
    const dynamicPanelsContainer = byId("dynamic-provider-panels");
    if (tabline && dynamicPanelsContainer) {
      const activeTabEl = tabline.querySelector(".tab.active");
      const currentTab = activeTabEl ? activeTabEl.dataset.provider : "omni";

      // Remove previously generated dynamic tabs
      tabline.querySelectorAll(".tab[data-dynamic='true']").forEach(function(el) {
        el.remove();
      });

      const builtinTabMap = {
        omniroute: byId("tab-omni"),
        jev: byId("tab-jev"),
        muse: byId("tab-muse"),
      };

      const dynamicModules = [];
      (modules || []).forEach(function(m) {
        if (builtinTabMap[m.id]) {
          tabline.appendChild(builtinTabMap[m.id]);
        } else {
          const btn = document.createElement("button");
          btn.className = "tab" + (currentTab === m.id ? " active" : "");
          btn.dataset.provider = m.id;
          btn.dataset.dynamic = "true";
          btn.setAttribute("role", "tab");
          btn.setAttribute("aria-selected", currentTab === m.id ? "true" : "false");
          btn.id = "tab-" + m.id;
          btn.textContent = m.name || m.id;
          tabline.appendChild(btn);
          dynamicModules.push(m);
        }
      });

      dynamicPanelsContainer.innerHTML = dynamicModules.map(function(m) {
        const cfg = moduleConfigs?.[m.id] || {};
        const schema = m.config_schema || {};
        const fields = schema.fields || [];
        const isCurrent = currentTab === m.id;
        const cfgUrl = m.config_url || ("/modules/" + m.id + "/config");
        const fieldsHtml = fields.map(function(f) {
          return renderDynamicConfigField(f, m, cfg);
        }).join("");

        const resolvedHarEndpoint = resolveDynamicHarEndpoint(schema, m);
        const harHtml = resolvedHarEndpoint
          ? ("<div class=\"field-group formgroup\">" +
              "<label for=\"dyn-har-" + escapeHtml(m.id) + "\">Or Import HAR Capture (.har / .json)</label>" +
              "<input type=\"file\" id=\"dyn-har-" + escapeHtml(m.id) + "\" class=\"dynamic-har-file\" data-module-id=\"" + escapeHtml(m.id) + "\" data-har-endpoint=\"" + escapeHtml(resolvedHarEndpoint) + "\" accept=\".har,.json\">" +
              "<div class=\"hint\">" + escapeHtml(schema.har_hint || ("Posts HAR to " + resolvedHarEndpoint)) + "</div>" +
            "</div>")
          : "";

        return "<div class=\"provider-panel" + (isCurrent ? "" : " is-hidden") + "\" id=\"panel-" + escapeHtml(m.id) + "\" role=\"tabpanel\" aria-labelledby=\"tab-" + escapeHtml(m.id) + "\">" +
          "<div class=\"card-header\">" +
            "<h2>" + escapeHtml(m.name) + " Configuration</h2>" +
            "<span class=\"tag ok\"><code>" + escapeHtml(m.namespace) + "</code></span>" +
          "</div>" +
          "<p class=\"card-sub\">Automatic UI configuration for module <code>" + escapeHtml(m.id) + "</code>. Secrets are masked and persisted securely.</p>" +
          "<form class=\"dynamic-module-config-form\" data-module-id=\"" + escapeHtml(m.id) + "\">" +
            "<div class=\"grid-2 formgrid\">" + fieldsHtml + "</div>" +
            harHtml +
            "<div class=\"btns\">" +
              "<button type=\"submit\" class=\"btn primary btn-primary\">Save " + escapeHtml(m.name) + " Settings</button>" +
              "<a class=\"btn\" href=\"" + escapeHtml(cfgUrl) + "\">Open Standalone UI Config (" + escapeHtml(cfgUrl) + ") ↗</a>" +
            "</div>" +
          "</form>" +
        "</div>";
      }).join("");
    }
  }

  document.addEventListener("submit", function(e) {
    const dynForm = e.target.closest(".dynamic-module-config-form");
    if (!dynForm) return;
    e.preventDefault();
    const modId = dynForm.dataset.moduleId;
    if (!modId) return;
    const harInput = dynForm.querySelector(".dynamic-har-file");
    const harFile = harInput?.files?.[0];
    if (harFile && harInput?.dataset.harEndpoint) {
      harFile.text().then(function(rawText) {
        return fetch(harInput.dataset.harEndpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: rawText
        });
      })
      .then(function(r) { return r.json().then(function(j) { return { ok: r.ok, json: j }; }); })
      .then(function(res) {
        if (!res.ok) {
          showToast(res.json?.error?.message || res.json?.message || "HAR import failed", true);
        } else {
          showToast(res.json?.message || ("Imported HAR for " + modId), false);
          loadOverview();
        }
      })
      .catch(function(err) {
        showToast("HAR import failed: " + err.message, true);
      })
      .finally(function() {
        if (harInput) harInput.value = "";
      });
      return;
    }

    const cfg = {};
    dynForm.querySelectorAll("[data-key]").forEach(function(inp) {
      const k = inp.dataset.key;
      const t = inp.dataset.type;
      if (t === "password") {
        if (inp.value?.trim()) cfg[k] = inp.value.trim();
      } else if (t === "boolean") {
        cfg[k] = inp.value === "true";
      } else if (t === "number") {
        const n = Number(inp.value);
        if (!Number.isNaN(n)) cfg[k] = n;
      } else {
        cfg[k] = inp.value.trim();
      }
    });

    fetch("/api/v1/dashboard/modules/config", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ id: modId, config: cfg })
    })
    .then(function(r) { return r.json().then(function(j) { return { ok: r.ok, json: j }; }); })
    .then(function(res) {
      if (!res.ok) {
        showToast(res.json.message || "Failed to save module config", true);
      } else {
        showToast(res.json.message || ("Saved configuration for " + modId), false);
        loadOverview();
      }
    })
    .catch(function(err) {
      showToast("Network error: " + err.message, true);
    });
  });

  window.removeModule = function(id) {
    sendModuleRequest(
      id,
      "/api/v1/dashboard/modules/remove",
      { id: id },
      "Removing personal module '" + id + "'...",
      "Personal module '" + id + "' was successfully removed.",
      "Failed to remove module"
    );
  };

  function clearInstallModuleInputs() {
    if (modSource) modSource.value = "";
    if (modZipFile) modZipFile.value = "";
    if (modId) modId.value = "";
    if (modName) modName.value = "";
    if (modNamespace) modNamespace.value = "";
    if (modEntrypoint) modEntrypoint.value = "";
    if (modCaps) modCaps.value = "";
  }

  function handleInstallModuleSuccess(moduleData) {
    const modIdText = moduleData?.id || "installed";
    modOkMsg.textContent = "Success! Module '" + modIdText + "' registered and ready.";
    setMessageVisible(modOkMsg, true);
    clearInstallModuleInputs();
    loadOverview();
  }

  function handleInstallModuleFailure(result) {
    const errorDetail = result.json?.message || ("HTTP " + result.status);
    modErrMsg.textContent = "Installation error: " + errorDetail;
    setMessageVisible(modErrMsg, true);
  }

  function submitCustomModuleInstall(payload) {
    installBtn.disabled = true;
    installBtn.textContent = "Installing...";

    postJson("/api/v1/dashboard/modules/install", payload)
      .then(function(result) {
        installBtn.disabled = false;
        installBtn.textContent = "Install Module";
        if (!result.ok) {
          handleInstallModuleFailure(result);
        } else {
          handleInstallModuleSuccess(result.json?.module);
        }
      })
      .catch(function(err) {
        installBtn.disabled = false;
        installBtn.textContent = "Install Module";
        modErrMsg.textContent = "Network error: " + err.message;
        setMessageVisible(modErrMsg, true);
      });
  }

  function buildModuleInstallPayload(sourceVal, zipBase64) {
    return {
      source: sourceVal || undefined,
      zip_base64: zipBase64 || undefined,
      id: modId?.value.trim() || undefined,
      name: modName?.value.trim() || undefined,
      namespace: modNamespace?.value.trim() || undefined,
      entrypoint: modEntrypoint?.value.trim() || undefined,
      capabilities: modCaps?.value.trim()
        ? modCaps.value.split(",").map(function(c) { return c.trim(); }).filter(Boolean)
        : undefined
    };
  }

  function handleZipFileAndInstall(zipFileInput, sourceVal) {
    const reader = new FileReader();
    reader.onload = function() {
      const base64 = reader.result.split(",")[1];
      submitCustomModuleInstall(buildModuleInstallPayload(sourceVal, base64));
    };
    reader.onerror = function() {
      installBtn.disabled = false;
      installBtn.textContent = "Install Module";
      modErrMsg.textContent = "Error reading selected file.";
      setMessageVisible(modErrMsg, true);
    };
    reader.readAsDataURL(zipFileInput);
  }

  // Custom module install form
  function initCustomInstallForm() {
    if (!installForm) return;
    installForm.addEventListener("submit", function(e) {
      e.preventDefault();
      setMessageVisible(modOkMsg, false);
      setMessageVisible(modErrMsg, false);

      const sourceVal = modSource ? modSource.value.trim() : "";
      const zipFileInput = modZipFile?.files?.[0] ?? null;

      if (!sourceVal && !zipFileInput) {
        modErrMsg.textContent = "Please specify a GitHub/ZIP URL or select a ZIP archive file to upload.";
        setMessageVisible(modErrMsg, true);
        return;
      }

      if (zipFileInput) {
        handleZipFileAndInstall(zipFileInput, sourceVal);
      } else {
        submitCustomModuleInstall(buildModuleInstallPayload(sourceVal, null));
      }
    });
  }

  function updateOmniOverviewCard(config, omni) {
    if (omniTarget) omniTarget.textContent = (omni.target_host || config.target_host || "127.0.0.1") + ":" + (omni.target_port || config.target_port || "20128");
    if (omniDesc) omniDesc.textContent = "Model: " + (omni.default_model || config.default_model || "gpt-5.6-sol");
  }

  function updateJevOverviewCard(config, jev) {
    const jevAccountsCount = (jev.accounts || []).length;
    if (jevStatus) {
      const suffix = jevAccountsCount === 1 ? "" : "s";
      jevStatus.textContent = jevAccountsCount > 0 ? `${jevAccountsCount} account${suffix}` : "Configured";
    }
    if (jevDesc) jevDesc.textContent = "Base URL: " + (config.jev_base_url || "https://api.jev.ai");
  }

  function updateOpenRouterOverviewCard(config, orJev) {
    const orAccountsCount = (orJev.accounts || []).length;
    if (openrouterStatus) {
      const suffix = orAccountsCount === 1 ? "" : "s";
      const fallback = orJev.enabled ? "Enabled" : "Disabled";
      openrouterStatus.textContent = orAccountsCount > 0 ? `${orAccountsCount} browser/key account${suffix}` : fallback;
    }
    if (openrouterDesc) openrouterDesc.textContent = "Model: " + (config.jev_openrouter_model || "anthropic/claude-3.5-sonnet");
  }

  function updateMuseOverviewCard(muse, approval) {
    if (museStatus) museStatus.textContent = muse.configured ? "Configured" : "Default Config";
    if (museDesc) museDesc.textContent = "Approval Mode: " + (approval.mode || "off");
  }

  function updateOverviewCards(config, providers) {
    const omni = providers.omniroute || {};
    const jev = providers.jev || {};
    const orJev = providers.openrouter_jev || {};
    const muse = providers.museai || {};
    const approval = muse.approval || {};

    updateOmniOverviewCard(config, omni);
    updateJevOverviewCard(config, jev);
    updateOpenRouterOverviewCard(config, orJev);
    updateMuseOverviewCard(muse, approval);
  }

  function populateOmniConfigForm(config) {
    if (cfgHost) cfgHost.value = config.target_host || "127.0.0.1";
    if (cfgPort) cfgPort.value = config.target_port || 20128;
    if (cfgDefaultModel) cfgDefaultModel.value = config.default_model || "gpt-5.6-sol";
    if (cfgTimeout) cfgTimeout.value = config.io_timeout_secs || 120;
    if (cfgFallbacks) cfgFallbacks.value = (config.fallbacks || []).join(", ");
  }

  function populateJevConfigForm(config, orJev) {
    if (cfgJevBase) cfgJevBase.value = config.jev_base_url || "https://api.jev.ai";
    if (cfgJevOrBase) cfgJevOrBase.value = config.jev_openrouter_base_url || "https://openrouter.ai/api/v1";
    if (cfgJevOrModel) cfgJevOrModel.value = config.jev_openrouter_model || "anthropic/claude-3.5-sonnet";
    if (cfgJevOrFallbackModels) cfgJevOrFallbackModels.value = (config.jev_openrouter_fallback_models || []).join(", ");
    if (cfgJevOrEnabled) cfgJevOrEnabled.value = (config.jev_openrouter_enabled !== false) ? "true" : "false";
    if (cfgJevCooldown) {
      const cooldown = orJev.credit_cooldown_secs != null ? orJev.credit_cooldown_secs : config.jev_credit_cooldown_secs;
      cfgJevCooldown.value = cooldown != null ? cooldown : 300;
    }
  }

  function setValue(el, value) {
    if (el) el.value = value;
  }

  function setMaskedHint(el, masked) {
    if (el) el.textContent = masked ? ("Current: " + masked) : "Current: not set";
  }

  function populateMuseConfigForm(config, muse, approval) {
    setValue(cfgMuseBase, config.museai_base_url || "https://muse.ai");
    setValue(cfgMuseWs, config.museai_ws_url || "wss://vm-438.metaaivm.com/");
    setValue(cfgMuseVmid, config.museai_vm_id || "vm-438");
    setMaskedHint(hintToken, muse.masked_access_token);
    setMaskedHint(hintNotary, muse.masked_notary_token);
    setMaskedHint(hintCookie, muse.masked_cookie);
    setValue(cfgMuseVideoPrompt, config.default_video_prompt || "");
    setValue(cfgMuseApprovalMode, approval.mode || "off");
    setValue(cfgMuseApprovalTimeout, approval.timeout_secs || 30);
    setValue(cfgMuseApprovalDestinations, (approval.allowed_destinations || []).join(", "));
    setValue(cfgMuseApprovalTypes, (approval.allowed_types || []).join(", "));
  }

  function populateProviderConfigForms(config, providers) {
    const orJev = providers.openrouter_jev || {};
    const muse = providers.museai || {};
    const approval = muse.approval || {};

    populateOmniConfigForm(config);
    populateJevConfigForm(config, orJev);
    populateMuseConfigForm(config, muse, approval);
  }

  function renderOverviewAccountsTable(providers) {
    const jev = providers.jev || {};
    const orJev = providers.openrouter_jev || {};
    const rows = [];
    (jev.accounts || []).forEach(function(acc) {
      rows.push({ provider: "Jev Native", label: acc.label || "default", secret: acc.masked_key, type: "API Key" });
    });
    (orJev.accounts || []).forEach(function(acc) {
      const isCookie = Boolean(acc.masked_cookie);
      rows.push({
        provider: "OpenRouter (Jev)",
        label: acc.label || "default",
        secret: isCookie ? acc.masked_cookie : acc.masked_api_key,
        type: isCookie ? "Session Cookie" : "API Key"
      });
    });

    if (countBadge) countBadge.textContent = rows.length + " account" + (rows.length === 1 ? "" : "s");
    if (tableBody) {
      if (!rows.length) {
        tableBody.innerHTML = "<tr><td colspan=\"4\" class=\"empty-hint\">No accounts registered in pool. Add keys below.</td></tr>";
      } else {
        tableBody.innerHTML = rows.map(function(r) {
          return "<tr>" +
            "<td><strong>" + escapeHtml(r.provider) + "</strong></td>" +
            "<td>" + escapeHtml(r.label) + "</td>" +
            "<td><code>" + escapeHtml(r.secret) + "</code></td>" +
            "<td><span class=\"pill gray\">" + escapeHtml(r.type) + "</span></td>" +
          "</tr>";
        }).join("");
      }
    }
  }

  // Load Overview Data
  function loadOverview() {
    fetch("/api/v1/dashboard/overview")
      .then(function(res) { return res.json(); })
      .then(function(data) {
        const config = data.config || {};
        const providers = data.providers || {};
        updateOverviewCards(config, providers);
        populateProviderConfigForms(config, providers);
        renderOverviewAccountsTable(providers);

        // Modules & Catalog
        allModules = data.modules || [];
        renderDynamicProviderMenus(allModules, data.module_configs || {});
        filterModules();
        renderCatalog(data.catalog || []);
      })
      .catch(function(err) {
        if (omniDesc) omniDesc.textContent = "Error loading overview: " + err.message;
      });
  }

  // OmniRoute Form Submission
  function initOmniRouteForm() {
    if (!cfgForm) return;
    cfgForm.addEventListener("submit", function(e) {
      e.preventDefault();
      setMessageVisible(cfgOkMsg, false);
      setMessageVisible(cfgErrMsg, false);
      if (saveCfgBtn) {
        saveCfgBtn.disabled = true;
        saveCfgBtn.textContent = "Saving...";
      }

      const fallbacksArray = cfgFallbacks ? cfgFallbacks.value.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [];
      const payload = {
        target_host: cfgHost ? cfgHost.value.trim() : undefined,
        target_port: cfgPort ? Number.parseInt(cfgPort.value, 10) : undefined,
        default_model: cfgDefaultModel ? cfgDefaultModel.value.trim() : undefined,
        io_timeout_secs: cfgTimeout ? Number.parseInt(cfgTimeout.value, 10) : undefined,
        fallbacks: fallbacksArray
      };

      postJson("/api/v1/dashboard/config", payload)
      .then(function(result) {
        if (saveCfgBtn) {
          saveCfgBtn.disabled = false;
          saveCfgBtn.textContent = "Save Gateway Settings";
        }
        if (!result.ok) {
          cfgErrMsg.textContent = result.json.message || ("Error updating settings (HTTP " + result.status + ")");
          setMessageVisible(cfgErrMsg, true);
        } else {
          cfgOkMsg.textContent = "Gateway configuration updated and saved.";
          setMessageVisible(cfgOkMsg, true);
          loadOverview();
        }
      })
      .catch(function(err) {
        if (saveCfgBtn) {
          saveCfgBtn.disabled = false;
          saveCfgBtn.textContent = "Save Gateway Settings";
        }
        cfgErrMsg.textContent = "Network error: " + err.message;
        setMessageVisible(cfgErrMsg, true);
      });
    });
  }

  // Muse Form Submission
  function initMuseForm() {
    if (!museForm) return;
    museForm.addEventListener("submit", function(e) {
      e.preventDefault();
      setMessageVisible(museOkMsg, false);
      setMessageVisible(museErrMsg, false);
      if (saveMuseBtn) {
        saveMuseBtn.disabled = true;
        saveMuseBtn.textContent = "Saving...";
      }

      const destinationsArray = cfgMuseApprovalDestinations ? cfgMuseApprovalDestinations.value.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [];
      const typesArray = cfgMuseApprovalTypes ? cfgMuseApprovalTypes.value.split(",").map(function(s) { return s.trim(); }).filter(Boolean) : [];

      const payload = {
        base_url: cfgMuseBase ? cfgMuseBase.value.trim() : undefined,
        ws_url: cfgMuseWs ? cfgMuseWs.value.trim() : undefined,
        vm_id: cfgMuseVmid ? cfgMuseVmid.value.trim() : undefined,
        access_token: cfgMuseToken?.value || undefined,
        notary_token: cfgMuseNotary?.value || undefined,
        cookie: cfgMuseCookie?.value || undefined,
        default_video_prompt: cfgMuseVideoPrompt ? cfgMuseVideoPrompt.value : undefined,
        approval: {
          mode: cfgMuseApprovalMode ? cfgMuseApprovalMode.value : undefined,
          timeout_secs: cfgMuseApprovalTimeout ? Number.parseInt(cfgMuseApprovalTimeout.value, 10) : undefined,
          allowed_destinations: destinationsArray,
          allowed_types: typesArray,
        }
      };

      postJson("/api/v1/dashboard/muse-config", payload)
      .then(function(result) {
        if (saveMuseBtn) {
          saveMuseBtn.disabled = false;
          saveMuseBtn.textContent = "Save Muse AI Settings";
        }
        if (!result.ok) {
          museErrMsg.textContent = result.json.message || ("Error updating Muse AI config (HTTP " + result.status + ")");
          setMessageVisible(museErrMsg, true);
        } else {
          if (cfgMuseToken) cfgMuseToken.value = "";
          if (cfgMuseNotary) cfgMuseNotary.value = "";
          if (cfgMuseCookie) cfgMuseCookie.value = "";
          museOkMsg.textContent = result.json.message || "Muse AI configuration updated and saved.";
          setMessageVisible(museOkMsg, true);
          loadOverview();
        }
      })
      .catch(function(err) {
        if (saveMuseBtn) {
          saveMuseBtn.disabled = false;
          saveMuseBtn.textContent = "Save Muse AI Settings";
        }
        museErrMsg.textContent = "Network error: " + err.message;
        setMessageVisible(museErrMsg, true);
      });
    });
  }

  // Jev Form Submission
  function initJevConfigForm() {
    if (!jevConfigForm) return;
    jevConfigForm.addEventListener("submit", function(e) {
      e.preventDefault();
      setMessageVisible(jevOkMsg, false);
      setMessageVisible(jevErrMsg, false);
      if (saveJevBtn) {
        saveJevBtn.disabled = true;
        saveJevBtn.textContent = "Saving...";
      }

      const payload = {
        jev_base_url: cfgJevBase ? cfgJevBase.value.trim() : undefined,
        jev_openrouter_base_url: cfgJevOrBase ? cfgJevOrBase.value.trim() : undefined,
        jev_openrouter_model: cfgJevOrModel ? cfgJevOrModel.value.trim() : undefined,
        jev_openrouter_fallback_models: cfgJevOrFallbackModels ? cfgJevOrFallbackModels.value.trim() : undefined,
        jev_openrouter_enabled: cfgJevOrEnabled ? (cfgJevOrEnabled.value === "true") : true,
        jev_credit_cooldown_secs: cfgJevCooldown?.value ? Number.parseInt(cfgJevCooldown.value, 10) : undefined,
      };

      postJson("/api/v1/dashboard/jev-config", payload)
      .then(function(result) {
        if (saveJevBtn) {
          saveJevBtn.disabled = false;
          saveJevBtn.textContent = "Save Jev & OpenRouter Settings";
        }
        if (!result.ok) {
          jevErrMsg.textContent = result.json.message || ("Error saving settings (HTTP " + result.status + ")");
          setMessageVisible(jevErrMsg, true);
        } else {
          jevOkMsg.textContent = "Success! Jev & OpenRouter configuration persisted.";
          setMessageVisible(jevOkMsg, true);
          loadOverview();
        }
      })
      .catch(function(err) {
        if (saveJevBtn) {
          saveJevBtn.disabled = false;
          saveJevBtn.textContent = "Save Jev & OpenRouter Settings";
        }
        jevErrMsg.textContent = "Network error: " + err.message;
        setMessageVisible(jevErrMsg, true);
      });
    });
  }

  // Accounts Form Submission
  function initAddAccountForm() {
    if (!addForm) return;
    addForm.addEventListener("submit", function(ev) {
      ev.preventDefault();
      setMessageVisible(okMsg, false);
      setMessageVisible(errMsg, false);

      const raw = keysInput ? keysInput.value.trim() : "";
      if (!raw) {
        errMsg.textContent = "Please enter at least one key or session cookie.";
        setMessageVisible(errMsg, true);
        return;
      }

      const keys = raw.split(/[\r\n,]+/).map(function(k) { return k.trim(); }).filter(Boolean);
      if (!keys.length) {
        errMsg.textContent = "No valid keys detected.";
        setMessageVisible(errMsg, true);
        return;
      }

      submitBtn.disabled = true;
      submitBtn.textContent = "Adding...";

      const selectedProvider = providerSelect?.value || "jev";
      const payload = {
        provider: selectedProvider,
        api_keys: keys,
        label: labelInput?.value.trim() || undefined
      };

      postJson("/api/v1/dashboard/accounts", payload)
      .then(function(result) {
        submitBtn.disabled = false;
        submitBtn.textContent = "Add to Account Pool";
        if (!result.ok) {
          errMsg.textContent = result.json.message || ("Error adding accounts (HTTP " + result.status + ")");
          setMessageVisible(errMsg, true);
        } else {
          if (keysInput) keysInput.value = "";
          if (labelInput) labelInput.value = "";
          okMsg.textContent = "Success! Added " + result.json.added_count + " new key(s). Total accounts in pool: " + result.json.total_accounts + ".";
          setMessageVisible(okMsg, true);
          loadOverview();
        }
      })
      .catch(function(err) {
        submitBtn.disabled = false;
        submitBtn.textContent = "Add to Account Pool";
        errMsg.textContent = "Network error: " + err.message;
        setMessageVisible(errMsg, true);
      });
    });
  }

  // App Shell Console Facade
  const GatewayConsole = {
    syncHashNavigation: function() {
      const hash = window.location.hash;
      const raw = (hash || "").replace(/^#/, "").trim().toLowerCase();
      const targetKey = normalizeRoute(hash);
      const tab = raw === "muse-settings" ? "muse" : undefined;
      navigateTo(targetKey, tab);
    },
    start: function() {
      document.documentElement.classList.add("ui-shell-ready");
      initToolbarAndNavigation();
      initModuleActionDialog();
      initCatalogGrid();
      initCustomInstallForm();
      initOmniRouteForm();
      initMuseForm();
      initJevConfigForm();
      initAddAccountForm();
      this.syncHashNavigation();
      window.addEventListener("hashchange", this.syncHashNavigation.bind(this));
      loadOverview();
    }
  };

  window.GatewayConsole = GatewayConsole;
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function() { GatewayConsole.start(); });
  } else {
    GatewayConsole.start();
  }
})();
