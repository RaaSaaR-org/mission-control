// MissionControl dashboard: progressive enhancements. Every page works without
// JS; editing needs it. Handlers are delegated from `document` so the main
// content can be swapped in place when files change on disk.
(function () {
  "use strict";
  var doc = document;
  var root = doc.documentElement;

  function store(kind) {
    return {
      get: function (k) { try { return window[kind].getItem(k); } catch (e) { return null; } },
      set: function (k, v) { try { window[kind].setItem(k, v); } catch (e) { /* storage blocked */ } },
      del: function (k) { try { window[kind].removeItem(k); } catch (e) { /* storage blocked */ } }
    };
  }
  var local = store("localStorage");
  var session = store("sessionStorage");

  function $(sel, ctx) { return (ctx || doc).querySelector(sel); }
  function $$(sel, ctx) { return Array.prototype.slice.call((ctx || doc).querySelectorAll(sel)); }
  function typing(el) {
    return el && (el.isContentEditable || /^(TEXTAREA|SELECT)$/.test(el.tagName) ||
      (el.tagName === "INPUT" && !/^(checkbox|radio|button|submit)$/.test(el.type)));
  }
  function el(html) {
    var t = doc.createElement("template");
    t.innerHTML = html.trim();
    return t.content.firstElementChild;
  }
  function svgIcon(name) {
    var ns = "http://www.w3.org/2000/svg";
    var svg = doc.createElementNS(ns, "svg");
    svg.setAttribute("class", "icon");
    svg.setAttribute("aria-hidden", "true");
    var use = doc.createElementNS(ns, "use");
    use.setAttribute("href", "#i-" + name);
    svg.appendChild(use);
    return svg;
  }
  function statusLabel(s) {
    s = String(s || "").replace(/-/g, " ");
    return s.charAt(0).toUpperCase() + s.slice(1);
  }
  function dialogOpen() { return !!$("dialog[open]"); }

  var search = $(".site-search");
  // Base path for reverse-proxy deployments, from the server-rewritten form action.
  var base = search ? search.getAttribute("action").replace(/\/search$/, "") : "";
  var mc = (window.mc = window.mc || {});
  mc.base = base;
  var editable = root.hasAttribute("data-edit");
  var main = $("#main");

  // ── Toasts ────────────────────────────────────────────────────────
  // mc.toast(message, tone = "positive", action = {label, onClick | href})
  var region = $(".toast-region");
  mc.toast = function (message, tone, action) {
    if (!region) return function () {};
    tone = tone || "positive";
    var t = doc.createElement("div");
    t.className = "toast tone-" + tone;
    if (tone === "negative") t.setAttribute("role", "alert");
    var lamp = doc.createElement("span");
    lamp.className = "lamp lamp-solid tone-" + tone;
    var text = doc.createElement("span");
    text.className = "toast-text";
    text.textContent = message;
    t.appendChild(lamp);
    t.appendChild(text);
    var timer = null;
    function close() {
      clearTimeout(timer);
      if (t.parentNode) t.parentNode.removeChild(t);
    }
    if (action) {
      var a = doc.createElement(action.href ? "a" : "button");
      a.className = "btn btn-ghost btn-sm toast-action";
      a.textContent = action.label;
      if (action.href) a.href = action.href; else a.type = "button";
      a.addEventListener("click", function (e) {
        if (action.onClick) { e.preventDefault(); action.onClick(); }
        close();
      });
      t.appendChild(a);
    }
    if (tone === "negative" || action) {
      var x = doc.createElement("button");
      x.type = "button";
      x.className = "icon-btn toast-close";
      x.setAttribute("aria-label", "Dismiss");
      x.appendChild(svgIcon("close"));
      x.addEventListener("click", close);
      t.appendChild(x);
    }
    region.appendChild(t);
    function arm() { if (tone !== "negative") timer = setTimeout(close, action ? 8000 : 4000); }
    t.addEventListener("mouseenter", function () { clearTimeout(timer); });
    t.addEventListener("mouseleave", arm);
    t.addEventListener("focusin", function () { clearTimeout(timer); });
    t.addEventListener("focusout", arm);
    arm();
    return close;
  };
  function closeNewestToast() {
    var all = $$(".toast", region);
    if (!all.length) return false;
    var last = all[all.length - 1];
    last.parentNode.removeChild(last);
    return true;
  }

  // ── API ───────────────────────────────────────────────────────────
  // Writes carry X-MC-Request; the server rejects writes without it.
  var known = null; // last repo version seen
  var writeSeq = 0;
  function api(method, path, body) {
    var opts = { method: method, credentials: "same-origin", headers: { Accept: "application/json" } };
    if (method !== "GET") opts.headers["X-MC-Request"] = "1";
    if (body !== undefined) {
      opts.headers["Content-Type"] = "application/json";
      opts.body = JSON.stringify(body);
    }
    return fetch(base + "/api" + path, opts).then(function (r) {
      return r.json().catch(function () { return {}; }).then(function (data) {
        if (!r.ok) {
          var err = new Error(data.message || "The server answered " + r.status + ".");
          err.status = r.status;
          err.field = data.field;
          throw err;
        }
        if (data.version) { known = data.version; writeSeq++; hideNotice(); invalidatePreviews(); }
        return data;
      });
    }, function () {
      throw new Error("Couldn't reach mc serve. Is it still running?");
    });
  }
  mc.api = api;

  // ── Theme ─────────────────────────────────────────────────────────
  var themeSwitch = $(".theme-switch");
  var themeLocked = false;
  function applyTheme(value) {
    if (value === "light" || value === "dark") {
      root.dataset.theme = value;
      local.set("mc-theme", value);
    } else {
      delete root.dataset.theme;
      local.del("mc-theme");
      value = "system";
    }
    if (!themeSwitch) return;
    $$("button", themeSwitch).forEach(function (b) {
      var on = b.dataset.themeValue === value;
      b.setAttribute("aria-checked", String(on));
      b.tabIndex = on ? 0 : -1;
    });
  }
  if (themeSwitch) {
    var scheme = getComputedStyle(root).colorScheme.trim();
    // A brand stylesheet that pins one colour scheme wins over the switch.
    themeLocked = (scheme === "light" || scheme === "dark") && root.dataset.theme !== scheme;
    if (themeLocked) root.dataset.theme = scheme;
    if (!themeLocked) {
      themeSwitch.hidden = false;
      applyTheme(root.dataset.theme || "system");
      themeSwitch.addEventListener("click", function (e) {
        var b = e.target.closest("button[data-theme-value]");
        if (b) applyTheme(b.dataset.themeValue);
      });
      themeSwitch.addEventListener("keydown", function (e) {
        if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
        var items = $$("button", themeSwitch);
        var i = items.indexOf(doc.activeElement);
        var next = items[(i + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length];
        next.focus();
        applyTheme(next.dataset.themeValue);
        e.preventDefault();
      });
    }
  }
  function toggleTheme() {
    if (themeLocked) {
      mc.toast("This brand sets the colour scheme.", "progress");
      return;
    }
    var dark = root.dataset.theme
      ? root.dataset.theme === "dark"
      : window.matchMedia("(prefers-color-scheme: dark)").matches;
    applyTheme(dark ? "light" : "dark");
  }
  mc.toggleTheme = toggleTheme;

  // ── Enhance: reveal JS-only controls (runs again after a soft refresh) ─
  var isMac = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  function enhance(scope) {
    if (!isMac) {
      $$("kbd", scope).forEach(function (k) {
        if (k.textContent.indexOf("⌘") !== -1) k.textContent = k.textContent.replace("⌘", "Ctrl ");
      });
    }
    $$(".density-toggle", scope).forEach(function (btn) {
      btn.hidden = false;
      btn.setAttribute("aria-pressed", String(root.classList.contains("density-compact")));
    });
    if (navigator.clipboard) $$(".copy-id", scope).forEach(function (b) { b.hidden = false; });
    if (!editable) return;
    $$("[data-new-task], [data-edit-open], .card-move", scope).forEach(function (b) { b.hidden = false; });
    $$(".kanban-item", scope).forEach(function (item) { item.draggable = true; });
    $$(".task-check[data-line]", scope).forEach(function (b) { b.disabled = false; });
    $$("[data-comment-form]", scope).forEach(function (f) {
      f.hidden = false;
      var section = f.closest("[data-comments]");
      if (section) section.hidden = false;
      var name = local.get("mc-comment-author");
      if (name && f.elements.author && !f.elements.author.value) f.elements.author.value = name;
    });
  }
  enhance(doc);

  // ── Delegated clicks ──────────────────────────────────────────────
  doc.addEventListener("click", function (e) {
    var t = e.target;
    if (!t.closest) return;
    var b;
    if ((b = t.closest("[data-new-task]"))) {
      newTask(b.dataset.status ? { status: b.dataset.status } : {});
    } else if ((b = t.closest(".card-move"))) {
      e.preventDefault();
      var item = b.closest(".kanban-item");
      if (menu && menuItem === item) closeMenu(); else openMenu(item, b);
    } else if ((b = t.closest(".copy-id"))) {
      var id = b.dataset.copy;
      navigator.clipboard.writeText(id).then(
        function () { mc.toast("Copied " + id); },
        function () { mc.toast("Couldn't copy. Select the ID and copy it manually.", "negative"); }
      );
    } else if ((b = t.closest(".density-toggle"))) {
      var on = root.classList.toggle("density-compact");
      if (on) local.set("mc-density", "compact"); else local.del("mc-density");
      $$(".density-toggle").forEach(function (d) { d.setAttribute("aria-pressed", String(on)); });
    } else if ((b = t.closest("[data-edit-open]"))) {
      toggleEdit(true);
    } else if ((b = t.closest("form[data-edit-task] [data-cancel]"))) {
      toggleEdit(false);
    } else if ((b = t.closest("dialog [data-close]"))) {
      b.closest("dialog").close();
    }
  });

  // ── Filters ───────────────────────────────────────────────────────
  // Dropdown filters apply on change; unset filters stay out of the URL.
  doc.addEventListener("change", function (e) {
    var select = e.target;
    if (!select.matches || !select.matches("select[data-autosubmit]")) return;
    Array.prototype.forEach.call(select.form.elements, function (f) {
      if (f.name && f.value === "") f.disabled = true;
    });
    select.form.submit();
  });

  // Live text filter over [data-row] items; on the board it also updates lane counts.
  function applyFilter(input) {
    var scope = doc.getElementById(input.getAttribute("data-filter"));
    if (!scope) return;
    var terms = input.value.toLowerCase().split(/\s+/).filter(Boolean);
    var shown = 0;
    $$("[data-row]", scope).forEach(function (row) {
      var text = row.textContent.toLowerCase();
      var hit = terms.every(function (t) { return text.indexOf(t) !== -1; });
      row.hidden = !hit;
      if (hit) shown++;
    });
    var count = $("[data-filter-count]");
    if (count) count.textContent = shown;
    $$(".kanban-column", scope).forEach(function (lane) {
      syncLane(lane);
      var more = $(".kanban-more", lane);
      if (more && terms.length) more.open = true;
    });
  }
  doc.addEventListener("input", function (e) {
    if (e.target.matches && e.target.matches("input[data-filter]")) applyFilter(e.target);
  });
  doc.addEventListener("keydown", function (e) {
    var input = e.target;
    if (e.key !== "Escape" || !input.matches || !input.matches("input[data-filter]") || !input.value) return;
    input.value = "";
    applyFilter(input);
    e.stopPropagation();
  }, true);

  // ── Recent entities (for the palette) ─────────────────────────────
  function readRecent() {
    try { return JSON.parse(session.get("mc-recent") || "[]").filter(Boolean); } catch (e) { return []; }
  }
  var detail = $(".detail[data-entity-id]");
  if (detail) {
    var entry = { id: detail.dataset.entityId, t: detail.dataset.entityTitle, k: detail.dataset.entityKind };
    var recent = [entry].concat(readRecent().filter(function (r) { return r.id !== entry.id; })).slice(0, 5);
    session.set("mc-recent", JSON.stringify(recent));
  }

  // ── Flight-plan sweep, once per session ───────────────────────────
  if ($(".flight-plan") && !session.get("mc-swept")) {
    session.set("mc-swept", "1");
    root.classList.add("mc-sweep");
    setTimeout(function () { root.classList.remove("mc-sweep"); }, 800);
  }

  // ── Command palette ───────────────────────────────────────────────
  var palette = $("dialog.palette");
  var palInput = palette && $("input", palette);
  var palList = palette && $(".pal-list", palette);
  var siteInput = $("#site-search");
  var palData = null;
  var palLoading = null;
  var palItems = [];
  var palSel = 0;
  var CLOSED = /^(done|completed|cancelled|canceled|archived)$/;

  function loadPalette() {
    if (!palLoading) {
      palLoading = api("GET", "/palette").then(function (d) { palData = d; return d; }, function (err) {
        palLoading = null;
        throw err;
      });
    }
    return palLoading;
  }
  function invalidatePalette() { palData = null; palLoading = null; }

  function focusSearch() {
    if (!siteInput) return;
    var toggle = $("#nav-toggle");
    if (toggle && siteInput.offsetParent === null) toggle.checked = true;
    siteInput.focus();
    siteInput.select();
  }

  mc.openPalette = function (q) {
    if (!palette || typeof palette.showModal !== "function") { focusSearch(); return; }
    if (palette.open) { palInput.select(); return; }
    if (dialogOpen()) return;
    closeMenu();
    palInput.value = q || "";
    palSel = 0;
    palette.showModal();
    palInput.focus();
    renderPalette();
    loadPalette().then(renderPalette, renderPalette);
  };

  function wordStart(text, term) {
    var i = text.indexOf(term);
    while (i > 0) {
      if (/[\s\-_/([:"'.,]/.test(text.charAt(i - 1))) return true;
      i = text.indexOf(term, i + 1);
    }
    return i === 0;
  }
  function fuzzy(text, term) {
    if (term.length < 2) return 0;
    var ti = 0, gaps = 0, last = -1;
    for (var i = 0; i < text.length && ti < term.length; i++) {
      if (text.charAt(i) === term.charAt(ti)) {
        if (last >= 0) gaps += i - last - 1;
        last = i;
        ti++;
      }
    }
    return ti < term.length ? 0 : Math.max(10, 90 - gaps * 3);
  }
  function scoreText(text, term) {
    if (text.indexOf(term) === 0) return 400;
    if (wordStart(text, term)) return 300;
    if (text.indexOf(term) !== -1) return 200;
    return fuzzy(text, term);
  }
  function scoreEntity(e, terms) {
    var id = e.id.toLowerCase();
    var title = String(e.t || "").toLowerCase();
    var tags = (e.tags || []).map(function (t) { return String(t).toLowerCase(); });
    var num = (id.match(/-(\d+)$/) || [])[1];
    var total = 0;
    for (var i = 0; i < terms.length; i++) {
      var term = terms[i], s = 0;
      if (id === term) s = 1000;
      else if (num && /^\d+$/.test(term) && parseInt(term, 10) === parseInt(num, 10)) s = 700;
      else if (id.indexOf(term) === 0) s = 500;
      else if (title.indexOf(term) !== -1 || wordStart(title, term)) s = scoreText(title, term);
      else if (tags.indexOf(term) !== -1) s = 260;
      else if (tags.some(function (t) { return t.indexOf(term) !== -1; })) s = 150;
      else if (id.indexOf(term) !== -1) s = 120;
      else s = fuzzy(title, term);
      if (!s) return 0;
      total += s;
    }
    if (CLOSED.test(e.s || "")) total -= 40;
    return Math.max(total, 1);
  }
  function scoreLabel(label, terms) {
    var text = label.toLowerCase(), total = 0;
    for (var i = 0; i < terms.length; i++) {
      var s = scoreText(text, terms[i]);
      if (!s) return 0;
      total += s;
    }
    return total;
  }

  function navPages() {
    var pages = $$(".main-nav a").map(function (a) {
      return { label: a.querySelector(".nav-label").textContent, href: a.getAttribute("href").slice(base.length) || "/", keys: a.dataset.go ? "g " + a.dataset.go : "" };
    });
    return pages;
  }
  function paletteActions() {
    var acts = [];
    if (editable && $("dialog.new-task")) acts.push({ label: "New task", keys: "c", icon: "plus", run: function () { newTask({}); } });
    if (!themeLocked && themeSwitch) acts.push({ label: "Switch light and dark", keys: "t", icon: "moon", run: toggleTheme });
    acts.push({ label: "Keyboard shortcuts", keys: "?", icon: "list", run: openSheet });
    acts.push({ label: "Reload data", keys: "", icon: "refresh", run: function () { location.reload(); } });
    return acts;
  }

  function buildPaletteItems(q) {
    var terms = q.toLowerCase().split(/\s+/).filter(Boolean);
    var groups = [];
    var pages = (palData && palData.pages) || navPages();
    var acts = paletteActions();
    if (!terms.length) {
      groups.push({ label: "Actions", items: acts.map(function (a) { return { type: "action", act: a }; }) });
      groups.push({ label: "Go to", items: pages.map(function (p) { return { type: "page", page: p }; }) });
      var rec = readRecent();
      if (rec.length) {
        groups.push({ label: "Recent", items: rec.map(function (r) {
          var full = palData && palData.entities.filter(function (e) { return e.id === r.id; })[0];
          return { type: "entity", e: full || { id: r.id, t: r.t, k: r.k, s: "", tone: "neutral" } };
        }) });
      }
      return { groups: groups, terms: terms };
    }
    var actHits = acts.map(function (a) { return { type: "action", act: a, score: scoreLabel(a.label, terms) }; })
      .filter(function (x) { return x.score > 0; });
    var pageHits = pages.map(function (p) { return { type: "page", page: p, score: scoreLabel(p.label, terms) }; })
      .filter(function (x) { return x.score > 0; });
    var byScore = function (a, b) { return b.score - a.score; };
    if (palData) {
      var kinds = {};
      palData.entities.forEach(function (e) {
        var s = scoreEntity(e, terms);
        if (!s) return;
        (kinds[e.k] = kinds[e.k] || []).push({ type: "entity", e: e, score: s });
      });
      var entityGroups = (palData.kinds || []).filter(function (k) { return kinds[k.k]; }).map(function (k) {
        var items = kinds[k.k].sort(byScore);
        return { label: k.label, best: items[0].score, items: items.slice(0, 8) };
      });
      // The kind holding the best match comes first.
      entityGroups.sort(function (a, b) { return b.best - a.best; });
      var budget = 40;
      entityGroups.forEach(function (g) {
        g.items = g.items.slice(0, Math.max(0, budget));
        budget -= g.items.length;
      });
      var bestEntity = entityGroups.length ? entityGroups[0].best : 0;
      var lead = [];
      if (pageHits.length) lead.push({ label: "Go to", best: pageHits.sort(byScore)[0].score, items: pageHits.slice(0, 5) });
      if (actHits.length) lead.push({ label: "Actions", best: actHits.sort(byScore)[0].score, items: actHits.slice(0, 4) });
      // Pages and actions lead when they start with the query or match at
      // least as well as the best entity.
      var leads = function (g) { return g.best >= 400 || g.best >= bestEntity; };
      lead.forEach(function (g) { if (leads(g)) groups.push(g); });
      groups = groups.concat(entityGroups.filter(function (g) { return g.items.length; }));
      lead.forEach(function (g) { if (!leads(g)) groups.push(g); });
    } else {
      if (pageHits.length) groups.push({ label: "Go to", items: pageHits.sort(byScore) });
      if (actHits.length) groups.push({ label: "Actions", items: actHits.sort(byScore) });
      if (palLoading) groups.push({ label: "Loading entities…", items: [] });
    }
    groups.push({ label: "", items: [{ type: "search", q: q }] });
    return { groups: groups, terms: terms };
  }

  function highlight(target, text, terms) {
    text = String(text || "");
    var lower = text.toLowerCase(), marks = [];
    terms.forEach(function (term) {
      var i = lower.indexOf(term);
      while (term && i !== -1) { marks.push([i, i + term.length]); i = lower.indexOf(term, i + term.length); }
    });
    marks.sort(function (a, b) { return a[0] - b[0]; });
    var pos = 0;
    marks.forEach(function (m) {
      if (m[0] < pos) { if (m[1] > pos) m[0] = pos; else return; }
      if (m[0] > pos) target.appendChild(doc.createTextNode(text.slice(pos, m[0])));
      var mark = doc.createElement("mark");
      mark.textContent = text.slice(m[0], m[1]);
      target.appendChild(mark);
      pos = m[1];
    });
    if (pos < text.length) target.appendChild(doc.createTextNode(text.slice(pos)));
  }
  function kbdKeys(keys) {
    var span = doc.createElement("span");
    span.className = "pal-keys";
    keys.split(" ").forEach(function (k) {
      var kb = doc.createElement("kbd");
      kb.textContent = k;
      span.appendChild(kb);
    });
    return span;
  }
  function shortDate(d) {
    var m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(d || "");
    if (!m) return "";
    var date = new Date(+m[1], +m[2] - 1, +m[3]);
    var opts = { day: "numeric", month: "short" };
    if (+m[1] !== new Date().getFullYear()) opts.year = "numeric";
    return date.toLocaleDateString("en-GB", opts);
  }

  function renderPalette() {
    if (!palette || !palette.open) return;
    var q = palInput.value.trim();
    var built = buildPaletteItems(q);
    palList.textContent = "";
    palItems = [];
    built.groups.forEach(function (g) {
      if (g.label) {
        var h = doc.createElement("li");
        h.className = "pal-group";
        h.setAttribute("role", "presentation");
        h.textContent = g.label;
        palList.appendChild(h);
      }
      g.items.forEach(function (item) {
        var li = doc.createElement("li");
        li.setAttribute("role", "option");
        li.id = "pal-opt-" + palItems.length;
        li.dataset.index = palItems.length;
        var lead = doc.createElement("span");
        lead.className = "pal-lead";
        var title = doc.createElement("span");
        title.className = "pal-title";
        var meta = doc.createElement("span");
        meta.className = "pal-meta";
        if (item.type === "entity") {
          var e = item.e;
          var lamp = doc.createElement("span");
          lamp.className = "lamp tone-" + (e.tone || "neutral") + (e.tone === "positive" ? " lamp-solid" : "");
          lamp.setAttribute("aria-hidden", "true");
          lead.appendChild(lamp);
          highlight(title, e.t || e.id, built.terms);
          if (e.s) {
            var st = doc.createElement("span");
            st.className = "sr-only";
            st.textContent = ", " + statusLabel(e.s);
            title.appendChild(st);
          }
          if (e.d) {
            var d = doc.createElement("time");
            d.dateTime = e.d;
            d.textContent = shortDate(e.d);
            meta.appendChild(d);
          }
          var idc = doc.createElement("span");
          idc.className = "entity-id";
          highlight(idc, e.id, built.terms);
          meta.appendChild(idc);
          item.href = "/entity/" + encodeURIComponent(e.id);
        } else if (item.type === "page") {
          lead.appendChild(svgIcon("chevron"));
          highlight(title, item.page.label, built.terms);
          if (item.page.keys) meta.appendChild(kbdKeys(item.page.keys));
          item.href = item.page.href;
        } else if (item.type === "action") {
          lead.appendChild(svgIcon(item.act.icon || "chevron"));
          highlight(title, item.act.label, built.terms);
          if (item.act.keys) meta.appendChild(kbdKeys(item.act.keys));
          item.run = item.act.run;
        } else {
          lead.appendChild(svgIcon("search"));
          title.textContent = "Search all notes for “" + item.q + "”";
          meta.appendChild(kbdKeys("↵"));
          item.href = "/search?q=" + encodeURIComponent(item.q);
        }
        li.appendChild(lead);
        li.appendChild(title);
        li.appendChild(meta);
        palList.appendChild(li);
        palItems.push(item);
      });
    });
    palSel = Math.min(palSel, Math.max(0, palItems.length - 1));
    selectPal(palSel, false);
  }
  function selectPal(i, scroll) {
    if (!palItems.length) { palInput.removeAttribute("aria-activedescendant"); return; }
    palSel = (i + palItems.length) % palItems.length;
    $$("[role=option]", palList).forEach(function (li) {
      li.setAttribute("aria-selected", String(+li.dataset.index === palSel));
    });
    var cur = doc.getElementById("pal-opt-" + palSel);
    palInput.setAttribute("aria-activedescendant", cur.id);
    if (scroll !== false) cur.scrollIntoView({ block: "nearest" });
  }
  function activatePal(i, newTab) {
    var item = palItems[i];
    if (!item) {
      var q = palInput.value.trim();
      if (q) location.href = base + "/search?q=" + encodeURIComponent(q);
      return;
    }
    if (item.run) { palette.close(); item.run(); return; }
    var url = base + (item.href === "/" && base ? "" : item.href);
    if (newTab) { window.open(url, "_blank", "noopener"); return; }
    palette.close();
    location.href = url;
  }
  if (palette) {
    palInput.addEventListener("input", function () { palSel = 0; renderPalette(); });
    palInput.addEventListener("keydown", function (e) {
      if (e.key === "ArrowDown" || (e.ctrlKey && e.key === "n")) { e.preventDefault(); selectPal(palSel + 1); }
      else if (e.key === "ArrowUp" || (e.ctrlKey && e.key === "p")) { e.preventDefault(); selectPal(palSel - 1); }
      else if (e.key === "Enter") { e.preventDefault(); activatePal(palSel, e.metaKey || e.ctrlKey); }
    });
    palList.addEventListener("mousemove", function (e) {
      var li = e.target.closest("[role=option]");
      if (li && +li.dataset.index !== palSel) selectPal(+li.dataset.index, false);
    });
    palList.addEventListener("click", function (e) {
      var li = e.target.closest("[role=option]");
      if (li) activatePal(+li.dataset.index, e.metaKey || e.ctrlKey);
    });
    // Clicking the backdrop closes the palette.
    palette.addEventListener("click", function (e) { if (e.target === palette) palette.close(); });
    if (siteInput) {
      siteInput.addEventListener("focus", function () {
        if (typeof palette.showModal !== "function") return;
        var q = siteInput.value;
        siteInput.blur();
        mc.openPalette(q);
      });
    }
    // Warm the cache once the page is idle so the first open is instant.
    (window.requestIdleCallback || function (f) { setTimeout(f, 1200); })(function () {
      loadPalette().catch(function () { /* retried on open */ });
    });
  }

  function openSheet() {
    var sheet = $("dialog.shortcut-sheet");
    if (sheet && !sheet.open && typeof sheet.showModal === "function" && !dialogOpen()) sheet.showModal();
  }

  // ── New task dialog ───────────────────────────────────────────────
  function setField(form, name, value) {
    var f = form.elements[name];
    if (!f || value == null) return;
    if (f.tagName === "SELECT" && !$$("option", f).some(function (o) { return o.value === value; })) return;
    f.value = value;
  }
  function clearErrors(form) {
    var box = $(".form-error", form);
    if (box) { box.hidden = true; box.textContent = ""; }
    $$("[aria-invalid]", form).forEach(function (f) { f.removeAttribute("aria-invalid"); });
  }
  function showError(form, err) {
    var box = $(".form-error", form);
    if (box) { box.textContent = err.message; box.hidden = false; }
    var f = err.field && form.elements[err.field];
    if (f) { f.setAttribute("aria-invalid", "true"); f.focus(); }
  }
  function busy(form, on) {
    var btn = $("button[type=submit]", form);
    if (btn) { btn.disabled = on; btn.setAttribute("aria-busy", String(on)); }
  }

  function newTask(preset) {
    var dialog = $("dialog.new-task");
    if (!editable || !dialog || typeof dialog.showModal !== "function") return;
    if (dialog.open) return;
    if (dialogOpen()) return;
    closeMenu();
    var form = $("form", dialog);
    form.reset();
    clearErrors(form);
    // Prefill from where the user is: a hub's detail page or the active filters.
    var hub = $(".detail[data-entity-kind]");
    if (hub && /^(project|customer|sprint)$/.test(hub.dataset.entityKind)) {
      setField(form, hub.dataset.entityKind, hub.dataset.entityId);
    }
    $$(".filter-form select").forEach(function (s) {
      if (s.value && /^(project|sprint|status|priority)$/.test(s.name)) setField(form, s.name, s.value);
      if (s.value && s.name === "owner") setField(form, "owner", s.value);
    });
    Object.keys(preset || {}).forEach(function (k) { setField(form, k, preset[k]); });
    dialog.showModal();
    form.elements.title.focus();
  }
  mc.newTask = newTask;

  function formValues(form) {
    var out = {};
    Array.prototype.forEach.call(form.elements, function (f) {
      if (!f.name || f.type === "checkbox" || f.type === "submit" || f.type === "button") return;
      out[f.name] = f.value.trim();
    });
    return out;
  }

  function createTask(form) {
    var v = formValues(form);
    if (!v.title) { form.elements.title.focus(); return; }
    var body = { title: v.title };
    ["status", "owner", "project", "customer", "sprint", "due_date"].forEach(function (k) {
      if (v[k]) body[k] = v[k];
    });
    if (v.priority) body.priority = parseInt(v.priority, 10);
    clearErrors(form);
    busy(form, true);
    api("POST", "/tasks", body).then(function (data) {
      busy(form, false);
      invalidatePalette();
      var id = data.task.id;
      var inserted = insertCard(data.html.card, data.task.status);
      if (form.elements.another && form.elements.another.checked) {
        form.elements.title.value = "";
        form.elements.title.focus();
      } else {
        form.closest("dialog").close();
      }
      if (inserted) {
        var link = $(".kanban-card", inserted);
        if (link && !dialogOpen()) link.focus();
      } else if (!$("#board")) {
        softRefresh(true);
      }
      mc.toast("Created " + id + ": " + data.task.title, "positive", { label: "Open", href: base + data.href });
    }, function (err) {
      busy(form, false);
      showError(form, err);
    });
  }

  doc.addEventListener("submit", function (e) {
    var form = e.target;
    if (form.matches("form[data-new-task-form]")) { e.preventDefault(); createTask(form); }
    else if (form.matches("form[data-edit-task]")) { e.preventDefault(); saveEdit(form); }
  });
  // ⌘↵ / Ctrl↵ submits edit forms from any field.
  doc.addEventListener("keydown", function (e) {
    if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey) || !e.target.form) return;
    var form = e.target.form;
    if (form.matches("form[data-new-task-form], form[data-edit-task], form[data-comment-form]")) {
      e.preventDefault();
      form.requestSubmit ? form.requestSubmit() : form.dispatchEvent(new Event("submit", { cancelable: true }));
    }
  });

  // ── Board: drag and drop, move menu ───────────────────────────────
  function laneFor(status) {
    var board = $("#board");
    if (!board) return null;
    return $$(".kanban-column", board).filter(function (l) { return l.dataset.status === status; })[0] || null;
  }
  function isDoneLane(lane) { return /^(done|completed)$/.test(lane.dataset.status); }
  function sortKey(item) {
    var num = parseInt((item.dataset.id.match(/(\d+)$/) || [0, 0])[1], 10);
    return [parseInt(item.dataset.pri || "3", 10), item.dataset.due || "9999-99-99", num];
  }
  function compareKeys(a, b) {
    for (var i = 0; i < a.length; i++) { if (a[i] < b[i]) return -1; if (a[i] > b[i]) return 1; }
    return 0;
  }
  function placeInLane(item, lane) {
    var cards = $(".kanban-cards", lane);
    var empty = $(".kanban-empty", cards);
    if (empty) empty.parentNode.removeChild(empty);
    if (isDoneLane(lane)) { cards.insertBefore(item, cards.firstChild); return; }
    var key = sortKey(item);
    var before = $$(":scope > .kanban-item", cards).filter(function (it) {
      return it !== item && compareKeys(sortKey(it), key) > 0;
    })[0] || null;
    cards.insertBefore(item, before);
  }
  function syncLane(lane) {
    if (!lane) return;
    var count = $("[data-lane-count]", lane);
    var visible = $$("[data-row]:not([hidden])", lane);
    if (count) count.textContent = visible.length;
    var head = $(".kanban-head", lane);
    var late = isDoneLane(lane) ? 0 : $$("[data-row]:not([hidden]) .kanban-card.is-overdue", lane).length;
    var lateEl = $(".kanban-late", head);
    if (late && !lateEl) {
      lateEl = doc.createElement("span");
      lateEl.className = "kanban-late";
      head.insertBefore(lateEl, $(".lane-add", head));
    }
    if (lateEl) { if (late) lateEl.textContent = late + " late"; else lateEl.parentNode.removeChild(lateEl); }
    var cards = $(".kanban-cards", lane);
    var empty = $(".kanban-empty", cards);
    if (!$$("[data-row]", lane).length && !empty) {
      empty = doc.createElement("div");
      empty.className = "kanban-empty";
      empty.textContent = "Nothing in " + statusLabel(lane.dataset.status).toLowerCase() + ".";
      cards.appendChild(empty);
    }
  }
  function insertCard(html, status) {
    var lane = laneFor(status);
    if (!lane || !html) return null;
    var item = el(html);
    item.draggable = editable;
    $$(".card-move", item).forEach(function (b) { b.hidden = false; });
    placeInLane(item, lane);
    syncLane(lane);
    item.classList.add("is-new");
    setTimeout(function () { item.classList.remove("is-new"); }, 1600);
    return item;
  }

  function moveTask(item, status, isUndo) {
    var id = item.dataset.id;
    var from = item.dataset.status;
    if (!status || status === from || item.classList.contains("is-pending")) return;
    var origParent = item.parentNode;
    var origNext = item.nextSibling;
    var fromLane = item.closest(".kanban-column");
    var toLane = laneFor(status);
    // Moving a node in the DOM drops its focus; keep it on the card.
    var hadFocus = item.contains(doc.activeElement);
    function refocus(it) { if (hadFocus && it.isConnected) $(".kanban-card", it).focus({ preventScroll: true }); }
    item.dataset.status = status;
    item.classList.add("is-pending");
    if (toLane) placeInLane(item, toLane);
    else if (item.parentNode) item.parentNode.removeChild(item);
    refocus(item);
    syncLane(fromLane);
    syncLane(toLane);
    api("POST", "/tasks/" + encodeURIComponent(id) + "/move", { status: status }).then(function (data) {
      item.classList.remove("is-pending");
      var current = item;
      if (item.parentNode && data.html && data.html.card) {
        hadFocus = item.contains(doc.activeElement);
        current = el(data.html.card);
        item.parentNode.replaceChild(current, item);
        current.draggable = true;
        $$(".card-move", current).forEach(function (b) { b.hidden = false; });
        refocus(current);
      }
      syncLane(toLane);
      invalidatePalette();
      var label = statusLabel(status);
      var msg = (isUndo ? "Moved " + id + " back to " : "Moved " + id + " to ") + label.toLowerCase();
      mc.toast(msg, "positive", isUndo ? null : {
        label: "Undo",
        onClick: function () {
          // A card moved off the board (e.g. cancelled) comes back to its lane.
          if (!current.parentNode && fromLane) placeInLane(current, fromLane);
          moveTask(current, from, true);
        }
      });
    }, function (err) {
      item.dataset.status = from;
      item.classList.remove("is-pending");
      if (item.parentNode) item.parentNode.removeChild(item);
      if (origParent) origParent.insertBefore(item, origNext && origNext.parentNode === origParent ? origNext : null);
      refocus(item);
      syncLane(fromLane);
      syncLane(toLane);
      mc.toast("Couldn't move " + id + ". " + err.message, "negative");
    });
  }
  mc.moveTask = function (id, status) {
    var item = $('.kanban-item[data-id="' + String(id).replace(/["\\]/g, "") + '"]');
    if (item) moveTask(item, status);
  };

  var dragItem = null;
  function clearDrop() {
    $$(".kanban-column.is-drop-target").forEach(function (l) { l.classList.remove("is-drop-target"); });
  }
  doc.addEventListener("dragstart", function (e) {
    if (!editable || !e.target.closest) return;
    var item = e.target.closest("#board .kanban-item");
    if (!item || item.classList.contains("is-pending")) return;
    dragItem = item;
    closeMenu();
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", item.dataset.id);
    var card = $(".kanban-card", item);
    if (card && e.dataTransfer.setDragImage) {
      var r = card.getBoundingClientRect();
      e.dataTransfer.setDragImage(card, e.clientX - r.left, e.clientY - r.top);
    }
    // Defer so the drag image is taken before the card dims.
    setTimeout(function () {
      if (dragItem !== item) return;
      item.classList.add("is-dragging");
      $("#board").classList.add("is-dragging");
    }, 0);
  });
  doc.addEventListener("dragover", function (e) {
    if (!dragItem || !e.target.closest) return;
    var lane = e.target.closest("#board .kanban-column");
    if (!lane) { clearDrop(); return; }
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    if (!lane.classList.contains("is-drop-target")) {
      clearDrop();
      if (lane.dataset.status !== dragItem.dataset.status) lane.classList.add("is-drop-target");
    }
  });
  doc.addEventListener("drop", function (e) {
    if (!dragItem) return;
    var lane = e.target.closest && e.target.closest("#board .kanban-column");
    if (!lane) return;
    e.preventDefault();
    var item = dragItem;
    endDrag();
    moveTask(item, lane.dataset.status);
  });
  function endDrag() {
    if (dragItem) dragItem.classList.remove("is-dragging");
    dragItem = null;
    clearDrop();
    var board = $("#board");
    if (board) board.classList.remove("is-dragging");
  }
  doc.addEventListener("dragend", endDrag);

  var menu = null, menuItem = null, menuBtn = null;
  function closeMenu(refocus) {
    if (!menu) return;
    menu.parentNode.removeChild(menu);
    if (menuBtn) {
      menuBtn.setAttribute("aria-expanded", "false");
      if (refocus && menuBtn.isConnected) menuBtn.focus();
    }
    menu = menuItem = menuBtn = null;
  }
  function openMenu(item, anchor) {
    var tpl = $("#move-menu-tpl");
    if (!tpl || !item) return;
    closeMenu();
    menu = tpl.content.firstElementChild.cloneNode(true);
    menuItem = item;
    menuBtn = anchor;
    var items = $$("[role=menuitemradio]", menu);
    items.forEach(function (b) {
      var on = b.dataset.status === item.dataset.status;
      b.setAttribute("aria-checked", String(on));
      b.tabIndex = -1;
    });
    var title = doc.createElement("div");
    title.className = "menu-title";
    title.textContent = "Move " + item.dataset.id + " to";
    menu.insertBefore(title, menu.firstChild);
    doc.body.appendChild(menu);
    var r = anchor.getBoundingClientRect();
    var w = menu.offsetWidth, h = menu.offsetHeight;
    var left = Math.max(8, Math.min(r.right - w, window.innerWidth - w - 8));
    var top = r.bottom + 4;
    if (top + h > window.innerHeight - 8) top = Math.max(8, r.top - h - 4);
    menu.style.left = left + "px";
    menu.style.top = top + "px";
    anchor.setAttribute("aria-expanded", "true");
    var current = items.filter(function (b) { return b.getAttribute("aria-checked") === "true"; })[0];
    (current || items[0]).focus();
    menu.addEventListener("keydown", function (e) {
      var i = items.indexOf(doc.activeElement);
      if (e.key === "ArrowDown") { e.preventDefault(); items[(i + 1) % items.length].focus(); }
      else if (e.key === "ArrowUp") { e.preventDefault(); items[(i - 1 + items.length) % items.length].focus(); }
      else if (e.key === "Home") { e.preventDefault(); items[0].focus(); }
      else if (e.key === "End") { e.preventDefault(); items[items.length - 1].focus(); }
      else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); closeMenu(true); }
      else if (e.key === "Tab") { closeMenu(true); e.preventDefault(); }
    });
    menu.addEventListener("click", function (e) {
      var b = e.target.closest("[role=menuitemradio]");
      if (!b) return;
      var target = menuItem;
      var card = $(".kanban-card", target);
      closeMenu();
      if (card) card.focus();
      moveTask(target, b.dataset.status);
    });
  }
  doc.addEventListener("mousedown", function (e) {
    if (menu && !menu.contains(e.target) && !(menuBtn && menuBtn.contains(e.target))) closeMenu();
  });
  window.addEventListener("resize", function () { closeMenu(); });
  window.addEventListener("scroll", function () { closeMenu(); }, true);

  // ── Task edit form ────────────────────────────────────────────────
  function toggleEdit(open) {
    var form = $("form[data-edit-task]");
    var btn = $("[data-edit-open]");
    if (!form || !editable) return;
    if (open) {
      form.hidden = false;
      if (btn) btn.setAttribute("aria-expanded", "true");
      clearErrors(form);
      var first = form.elements[0];
      if (first) first.focus();
      form.scrollIntoView({ block: "nearest" });
    } else {
      form.reset();
      clearErrors(form);
      form.hidden = true;
      if (btn) { btn.setAttribute("aria-expanded", "false"); btn.focus(); }
    }
  }
  function commitDefaults(form) {
    Array.prototype.forEach.call(form.elements, function (f) {
      if (f.tagName === "SELECT") $$("option", f).forEach(function (o) { o.defaultSelected = o.selected; });
      else if (f.name) f.defaultValue = f.value;
    });
  }
  function saveEdit(form) {
    var id = form.dataset.editTask;
    var body = {}, changed = 0;
    Array.prototype.forEach.call(form.elements, function (f) {
      if (!f.name) return;
      var initial = f.tagName === "SELECT"
        ? ($$("option", f).filter(function (o) { return o.defaultSelected; })[0] || f.options[0] || {}).value
        : f.defaultValue;
      if (f.value.trim() === String(initial || "").trim()) return;
      body[f.name] = f.name === "priority" ? parseInt(f.value, 10) : f.value.trim();
      changed++;
    });
    if (!changed) { toggleEdit(false); return; }
    clearErrors(form);
    busy(form, true);
    api("PATCH", "/tasks/" + encodeURIComponent(id), body).then(function (data) {
      busy(form, false);
      var block = $(".title-block");
      if (block && data.html.title_block) block.replaceWith(el(data.html.title_block));
      var rail = $(".detail-sidebar");
      if (rail && data.html.rail) rail.replaceWith(el(data.html.rail));
      var newBlock = $(".title-block");
      if (newBlock) {
        newBlock.classList.add("is-updated");
        setTimeout(function () { newBlock.classList.remove("is-updated"); }, 1600);
      }
      commitDefaults(form);
      invalidatePalette();
      toggleEdit(false);
      mc.toast("Saved " + id);
    }, function (err) {
      busy(form, false);
      showError(form, err);
    });
  }
  doc.addEventListener("keydown", function (e) {
    if (e.key !== "Escape") return;
    var form = e.target.closest && e.target.closest("form[data-edit-task]");
    if (form && !form.hidden) { e.preventDefault(); toggleEdit(false); }
  });

  // ── Checklists ────────────────────────────────────────────────────
  // Ticking a box in the notes writes it to the file. The box carries its
  // line and text, so the server refuses (409) if the file changed meanwhile.
  function checkProgress(body) {
    var bar = $("[data-check-progress]");
    if (!bar || !body) return;
    var boxes = $$(".task-check", body);
    var done = boxes.filter(function (b) { return b.checked; }).length;
    var total = boxes.length;
    var pct = total ? Math.floor(done * 100 / total) : 0;
    $("[data-check-done]", bar).textContent = done;
    $("[data-check-total]", bar).textContent = total;
    var meter = $(".progress", bar);
    meter.setAttribute("aria-valuenow", pct);
    meter.setAttribute("aria-label", done + " of " + total + " done");
    meter.classList.toggle("has-bead", pct > 0 && pct < 100);
    $(".progress-fill", meter).style.width = pct + "%";
  }
  doc.addEventListener("change", function (e) {
    var box = e.target;
    if (!editable || !box.matches || !box.matches(".task-check[data-line]")) return;
    var article = box.closest("[data-entity-id]");
    if (!article) return;
    var want = box.checked;
    var item = box.closest("li");
    var body = box.closest(".detail-body");
    box.disabled = true;
    if (item) item.classList.add("is-pending");
    checkProgress(body);
    api("POST", "/entities/" + encodeURIComponent(article.getAttribute("data-entity-id")) + "/checks", {
      line: Number(box.getAttribute("data-line")),
      checked: want,
      text: box.getAttribute("data-text")
    }).then(null, function (err) {
      box.checked = !want;
      checkProgress(body);
      mc.toast(err.message, "negative", err.status === 409
        ? { label: "Reload", onClick: function () { location.reload(); } }
        : undefined);
    }).then(function () {
      box.disabled = false;
      if (item) item.classList.remove("is-pending");
    });
  });

  // ── Comments ──────────────────────────────────────────────────────
  function postComment(form) {
    var article = form.closest("[data-entity-id]");
    var text = form.elements.text.value;
    if (!article || !text.trim()) { form.elements.text.focus(); return; }
    var author = form.elements.author.value.trim();
    clearErrors(form);
    busy(form, true);
    api("POST", "/entities/" + encodeURIComponent(article.getAttribute("data-entity-id")) + "/comments", {
      text: text,
      author: author || undefined
    }).then(function (data) {
      busy(form, false);
      if (author) local.set("mc-comment-author", author); else local.del("mc-comment-author");
      var section = form.closest("[data-comments]");
      var list = $(".comment-list", section);
      var li = el(data.html);
      li.classList.add("is-new");
      list.appendChild(li);
      setTimeout(function () { li.classList.remove("is-new"); }, 1600);
      $("[data-comment-count]", section).textContent = data.count;
      form.elements.text.value = "";
      mc.toast("Comment added");
    }, function (err) {
      busy(form, false);
      showError(form, err);
    });
  }
  doc.addEventListener("submit", function (e) {
    var form = e.target;
    if (form.matches("form[data-comment-form]")) { e.preventDefault(); if (editable) postComment(form); }
  });

  // ── Row cursor (j / k) ────────────────────────────────────────────
  var cursor = -1;
  function cursorRows() {
    return $$(".data-table tbody tr[data-row]:not([hidden]), .search-results > li[data-row]:not([hidden])");
  }
  function moveCursor(step) {
    var board = $("#board");
    if (board) {
      var cards = $$(".kanban-item:not([hidden]) > .kanban-card", board).filter(function (c) { return c.offsetParent; });
      if (!cards.length) return;
      var i = cards.indexOf(doc.activeElement);
      cards[i === -1 ? 0 : Math.max(0, Math.min(cards.length - 1, i + step))].focus();
      return;
    }
    var rows = cursorRows();
    if (!rows.length) return;
    rows.forEach(function (r) { r.classList.remove("is-cursor"); });
    cursor = cursor === -1 ? 0 : Math.max(0, Math.min(rows.length - 1, cursor + step));
    rows[cursor].classList.add("is-cursor");
    rows[cursor].scrollIntoView({ block: "nearest" });
  }
  function openCursor() {
    var row = cursorRows()[cursor];
    var link = row && $("a.name-link, a", row);
    if (link) { link.click(); return true; }
    return false;
  }

  // ── Keyboard shortcuts ────────────────────────────────────────────
  var chord = null, chordTimer = null;
  function go(path) { location.href = base + path; }
  function goKey(k) {
    if (k === "o") k = "d";
    if (k === "l") {
      if ($('.main-nav a[data-go="t"]')) go("/tasks/list");
      return;
    }
    var link = $('.main-nav a[data-go="' + k + '"]');
    if (link) location.href = link.getAttribute("href");
  }
  doc.addEventListener("keydown", function (e) {
    if (e.defaultPrevented || e.isComposing) return;
    var mod = e.metaKey || e.ctrlKey;
    if (mod && !e.altKey && !e.shiftKey && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      if (palette && palette.open) palette.close(); else mc.openPalette();
      return;
    }
    if (e.key === "Escape") {
      if (dialogOpen()) return;
      if (menu) { closeMenu(true); return; }
      if (cursor !== -1) {
        cursorRows().forEach(function (r) { r.classList.remove("is-cursor"); });
        cursor = -1;
        return;
      }
      closeNewestToast();
      return;
    }
    if (mod || typing(doc.activeElement) || dialogOpen() || menu) return;
    // Calendar: ← → or [ ] page months, . jumps to today. [ and ] need
    // Option on German Mac layouts, so Alt is allowed for them.
    var calKey = { ArrowLeft: "prev", "[": "prev", ArrowRight: "next", "]": "next", ".": "today" }[e.key];
    var calLink = calKey && (!e.altKey || e.key === "[" || e.key === "]") && !e.shiftKey && $("[data-cal-" + calKey + "]");
    if (calLink) {
      e.preventDefault();
      location.href = calLink.getAttribute("href");
      return;
    }
    if (e.altKey) return;
    var k = e.key;
    if (chord === "g") {
      chord = null;
      clearTimeout(chordTimer);
      e.preventDefault();
      goKey(k.toLowerCase());
      return;
    }
    if (k === "Enter" && cursor !== -1 && (doc.activeElement === doc.body || doc.activeElement === main)) {
      if (openCursor()) e.preventDefault();
      return;
    }
    var focusedItem = doc.activeElement && doc.activeElement.closest && doc.activeElement.closest(".kanban-item");
    switch (k) {
      case "/": e.preventDefault(); mc.openPalette(); break;
      case "?": e.preventDefault(); openSheet(); break;
      case "g":
        chord = "g";
        clearTimeout(chordTimer);
        chordTimer = setTimeout(function () { chord = null; }, 1000);
        break;
      case "c":
        if (editable && $("dialog.new-task")) { e.preventDefault(); newTask({}); }
        break;
      case "t": toggleTheme(); break;
      case "e":
        if (editable && $("form[data-edit-task]")) { e.preventDefault(); toggleEdit(true); }
        break;
      case "m":
        if (editable && focusedItem) {
          e.preventDefault();
          var btn = $(".card-move", focusedItem);
          if (btn) openMenu(focusedItem, btn);
        }
        break;
      case "b": case "l":
        var target = $('.view-toggle a[data-view-key="' + k + '"]');
        if (target && !target.classList.contains("active")) location.href = target.getAttribute("href");
        break;
      case "j": moveCursor(1); break;
      case "k": moveCursor(-1); break;
    }
  });

  // ── Entity previews ───────────────────────────────────────────────
  // Resting the mouse on a link to an entity, or focusing it from the
  // keyboard, shows a summary card from /api/preview/<id>. The card stays
  // open while the pointer is on it. Touch taps just navigate.
  var ENTITY_PATH = /^\/entity\/([^\/?#]+)$/;
  var PREVIEW_DELAY = 250, PREVIEW_SWITCH = 100, PREVIEW_GRACE = 180;
  var previews = {}; // id -> Promise of the card's HTML
  var pop = null, popLink = null, armed = null, quiet = null;
  var popTimer = null, hideTimer = null;
  function invalidatePreviews() { previews = {}; }
  function popShown() { return !!pop && !pop.hidden; }
  function previewId(a) {
    if (!a || a.origin !== location.origin) return null;
    if (a.closest(".preview-pop, .main-nav, .breadcrumb, dialog, .menu")) return null;
    var path = a.pathname;
    if (base && path.indexOf(base + "/") === 0) path = path.slice(base.length);
    var m = ENTITY_PATH.exec(path);
    if (!m) return null;
    var id;
    try { id = decodeURIComponent(m[1]); } catch (e) { return null; }
    var here = $("article.detail[data-entity-id]");
    return here && here.dataset.entityId === id ? null : id;
  }
  function loadPreview(id) {
    if (!previews[id]) {
      var p = fetch(base + "/api/preview/" + encodeURIComponent(id), { credentials: "same-origin", headers: { Accept: "text/html" } })
        .then(function (r) {
          if (!r.ok && r.status !== 404) throw new Error(String(r.status));
          return r.text();
        });
      p.catch(function () { if (previews[id] === p) delete previews[id]; });
      previews[id] = p;
    }
    return previews[id];
  }
  function ensurePop() {
    if (pop) return pop;
    pop = doc.createElement("div");
    pop.className = "preview-pop";
    pop.id = "mc-preview";
    pop.setAttribute("role", "tooltip");
    pop.hidden = true;
    pop.addEventListener("pointerleave", function (e) { if (e.pointerType !== "touch") scheduleHide(); });
    doc.body.appendChild(pop);
    return pop;
  }
  function placePreview(a, x, y) {
    // A link wrapped over several lines: anchor to the line under the
    // pointer, or to the whole link when it has keyboard focus.
    var r = a.getBoundingClientRect();
    if (x != null) {
      Array.prototype.forEach.call(a.getClientRects(), function (q) {
        if (y >= q.top && y <= q.bottom && x >= q.left && x <= q.right) r = q;
      });
    }
    var w = pop.offsetWidth, h = pop.offsetHeight, gap = 6, m = 8;
    var vw = root.clientWidth, vh = window.innerHeight;
    var above = r.bottom + gap + h > vh - m && r.top - gap - h >= m;
    var top = above ? r.top - gap - h : r.bottom + gap;
    pop.style.top = Math.max(m, Math.min(top, vh - h - m)) + "px";
    pop.style.left = Math.max(m, Math.min(r.left, vw - w - m)) + "px";
    pop.dataset.side = above ? "above" : "below";
  }
  function showPreview(a, id, x, y) {
    loadPreview(id).then(function (html) {
      if (armed !== a || !a.isConnected) return; // moved on meanwhile
      ensurePop();
      if (popLink && popLink !== a) popLink.removeAttribute("aria-describedby");
      pop.innerHTML = html;
      // A tooltip: its links are for the mouse; keyboard users press Enter.
      $$("a, button", pop).forEach(function (l) { l.tabIndex = -1; });
      pop.hidden = false;
      popLink = a;
      a.setAttribute("aria-describedby", pop.id);
      placePreview(a, x, y);
    }, function () { /* no card when the server is unreachable */ });
  }
  function hidePreview() {
    clearTimeout(popTimer);
    clearTimeout(hideTimer);
    armed = null;
    if (popLink) popLink.removeAttribute("aria-describedby");
    popLink = null;
    if (pop) { pop.hidden = true; pop.innerHTML = ""; }
  }
  function scheduleHide() {
    clearTimeout(hideTimer);
    hideTimer = setTimeout(hidePreview, PREVIEW_GRACE);
  }
  function previewBusy() { return dialogOpen() || menu || dragItem; }
  // Point the preview at link `a` (or nothing), after a short rest.
  function arm(a, id, x, y, delay) {
    if (a) {
      clearTimeout(hideTimer);
      if (a === armed) return;
      armed = a;
      clearTimeout(popTimer);
      if (a === popLink && popShown()) return;
      loadPreview(id).catch(function () {}); // fetch while we wait
      popTimer = setTimeout(function () { showPreview(a, id, x, y); }, popShown() ? PREVIEW_SWITCH : delay);
    } else if (armed) {
      armed = null;
      clearTimeout(popTimer);
      if (popShown()) scheduleHide();
    }
  }
  function linkUnder(e) {
    var a = e.target.closest && e.target.closest("a[href]");
    var id = previewId(a);
    if (!id || previewBusy()) return null;
    // Stretched row links cover the whole table row; only their text counts.
    var inside = Array.prototype.some.call(a.getClientRects(), function (q) {
      return e.clientX >= q.left - 1 && e.clientX <= q.right + 1 && e.clientY >= q.top - 1 && e.clientY <= q.bottom + 1;
    });
    if (!inside) return null;
    // The card replaces the native tooltip.
    if (a.title) { a.dataset.title = a.title; a.removeAttribute("title"); }
    return { a: a, id: id };
  }
  doc.addEventListener("pointermove", function (e) {
    if (e.pointerType === "touch") return;
    if (pop && pop.contains(e.target)) { clearTimeout(hideTimer); return; }
    var hit = linkUnder(e);
    if (hit && hit.a === quiet) return;
    quiet = null;
    if (hit) arm(hit.a, hit.id, e.clientX, e.clientY, PREVIEW_DELAY); else arm(null);
  }, { passive: true });
  doc.addEventListener("pointerout", function (e) { if (!e.relatedTarget) arm(null); });
  doc.addEventListener("pointerdown", function (e) {
    if (pop && pop.contains(e.target)) return;
    // Clicking the link navigates; don't pop the card up again meanwhile.
    if (armed) quiet = armed;
    hidePreview();
  });
  doc.addEventListener("focusin", function (e) {
    var a = e.target;
    if (a.tagName !== "A" || !a.matches(":focus-visible") || previewBusy()) return;
    var id = previewId(a);
    if (!id) return;
    if (a.title) { a.dataset.title = a.title; a.removeAttribute("title"); }
    arm(a, id, null, null, PREVIEW_DELAY + 100);
  });
  doc.addEventListener("focusout", function (e) {
    if (e.target === armed || e.target === popLink) hidePreview();
  });
  // Any key closes the card (it would cover what the key opens); Escape
  // closes only the card, and keeps it closed until the pointer moves on.
  doc.addEventListener("keydown", function (e) {
    if (!(popShown() || armed) || /^(Shift|Control|Alt|Meta)$/.test(e.key)) return;
    quiet = popLink || armed;
    var shown = popShown();
    hidePreview();
    if (e.key === "Escape" && shown) e.preventDefault();
  }, true);
  window.addEventListener("scroll", function (e) {
    if (!(pop && pop.contains(e.target))) hidePreview();
  }, true);
  window.addEventListener("resize", hidePreview);

  // ── Live refresh ──────────────────────────────────────────────────
  // Poll a fingerprint of the repo's files; when something else changes them,
  // refresh the page content in place, or offer a reload if that's unsafe.
  var notice = null, noticeKind = null, failures = 0, lastAutoToast = 0;
  function hideNotice(kind) {
    if (notice && (!kind || noticeKind === kind)) { notice.parentNode.removeChild(notice); notice = noticeKind = null; }
  }
  function showNotice(kind) {
    if (noticeKind === kind) return;
    hideNotice();
    var offline = kind === "offline";
    notice = doc.createElement("div");
    notice.className = "live-notice";
    notice.setAttribute("role", "status");
    var lamp = doc.createElement("span");
    lamp.className = "lamp lamp-solid tone-" + (offline ? "negative" : "progress");
    var text = doc.createElement("span");
    text.textContent = offline ? "Lost connection to mc serve." : "Files changed on disk.";
    notice.appendChild(lamp);
    notice.appendChild(text);
    if (!offline) {
      var reload = doc.createElement("button");
      reload.type = "button";
      reload.className = "btn btn-primary btn-sm";
      reload.textContent = "Reload";
      reload.addEventListener("click", function () { location.reload(); });
      notice.appendChild(reload);
    }
    var x = doc.createElement("button");
    x.type = "button";
    x.className = "icon-btn";
    x.setAttribute("aria-label", "Dismiss");
    x.appendChild(svgIcon("close"));
    x.addEventListener("click", function () { hideNotice(); });
    notice.appendChild(x);
    doc.body.appendChild(notice);
    noticeKind = kind;
  }
  function busyUi() {
    var active = doc.activeElement;
    var editForm = $("form[data-edit-task]");
    return dialogOpen() || menu || dragItem ||
      (editForm && !editForm.hidden) ||
      (active && typing(active) && main && main.contains(active)) ||
      $$("[data-comment-form] textarea").some(function (t) { return t.value.trim() !== ""; }) ||
      $(".is-pending");
  }
  function softRefresh(quiet) {
    if (!main) return;
    fetch(location.href, { credentials: "same-origin", cache: "no-store", headers: { Accept: "text/html" } })
      .then(function (r) { if (!r.ok) throw new Error(); return r.text(); })
      .then(function (html) {
        if (busyUi()) { showNotice("changed"); return; }
        var next = new DOMParser().parseFromString(html, "text/html");
        var nextMain = next.getElementById("main");
        if (!nextMain) throw new Error();
        var filters = {};
        $$("input[data-filter]", main).forEach(function (i) { filters[i.getAttribute("data-filter")] = i.value; });
        var openDetails = $$("details", main).map(function (d) { return d.open; });
        main.innerHTML = nextMain.innerHTML;
        var nav = $(".main-nav"), nextNav = next.querySelector(".main-nav");
        if (nav && nextNav) nav.innerHTML = nextNav.innerHTML;
        // The new-task dialog's lists (owners, sprints, ...) may have changed.
        var dlg = $("dialog.new-task"), nextDlg = next.querySelector("dialog.new-task");
        if (dlg && nextDlg && !dlg.open) dlg.replaceWith(doc.importNode(nextDlg, true));
        var dl = $("#mc-owners"), nextDl = next.getElementById("mc-owners");
        if (dl && nextDl) dl.replaceWith(doc.importNode(nextDl, true));
        $$("details", main).forEach(function (d, i) { if (openDetails[i]) d.open = true; });
        enhance(main);
        enhance($("dialog.new-task") || doc.createElement("div"));
        $$("input[data-filter]", main).forEach(function (i) {
          var v = filters[i.getAttribute("data-filter")];
          if (v) { i.value = v; applyFilter(i); }
        });
        cursor = -1;
        invalidatePalette();
        hidePreview();
        invalidatePreviews();
        hideNotice("changed");
        var now = Date.now();
        if (!quiet && now - lastAutoToast > 30000) {
          lastAutoToast = now;
          mc.toast("Updated with changes from disk.", "progress");
        }
      })
      .catch(function () { showNotice("changed"); });
  }
  mc.softRefresh = softRefresh;
  function poll() {
    if (doc.hidden || !window.fetch) return;
    var seq = writeSeq;
    fetch(base + "/api/version", { credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" } })
      .then(function (r) { if (!r.ok) throw new Error(); return r.json(); })
      .then(function (d) {
        failures = 0;
        hideNotice("offline");
        if (seq !== writeSeq) return; // our own write landed meanwhile
        if (known === null) { known = d.version; return; }
        if (d.version === known) {
          // A change seen while busy is applied once the page is idle again.
          if (noticeKind === "changed" && !busyUi()) softRefresh(true);
          return;
        }
        known = d.version;
        if (busyUi()) showNotice("changed"); else softRefresh(false);
      }, function () {
        failures++;
        if (failures >= 2) showNotice("offline");
      });
  }
  if (main && window.fetch) {
    poll();
    setInterval(poll, 4000);
    doc.addEventListener("visibilitychange", function () { if (!doc.hidden) poll(); });
  }
})();
