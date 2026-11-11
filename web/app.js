// The $ROT token page.
//
// Reads contract state over plain JSON-RPC - no wallet, no bundler, no
// dependencies, so this file runs identically from a static host or off your
// filesystem. Between polls it recomputes balances locally with RotModel,
// which is why the numbers fall while you watch instead of waiting for someone
// to send a transaction.

(function () {
  "use strict";

  const M = window.RotModel;
  const CFG = window.ROT_CONFIG;
  const $ = (id) => document.getElementById(id);

  const state = {
    key: CFG.default,
    contract: null,
    network: "testnet",
    rpcIndex: 0,
    config: null,
    stats: null,
    epochs: [],
    events: [],
    accounts: [],
    account: null,
    accountId: "",
    lastBalance: null,
    live: false,
  };

  // ---------------------------------------------------------------- RPC

  async function rpc(method, args) {
    const endpoints = CFG.rpc[state.network] || CFG.rpc.testnet;
    let lastErr;
    for (let i = 0; i < endpoints.length; i++) {
      const url = endpoints[(state.rpcIndex + i) % endpoints.length];
      try {
        const res = await fetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            jsonrpc: "2.0",
            id: "rot",
            method: "query",
            params: {
              request_type: "call_function",
              finality: "optimistic",
              account_id: state.contract,
              method_name: method,
              args_base64: btoa(JSON.stringify(args || {})),
            },
          }),
        });
        const json = await res.json();
        if (json.error) throw new Error(describeRpcError(json.error));
        const bytes = json.result && json.result.result;
        if (!bytes) throw new Error("empty result from " + method);
        state.rpcIndex = (state.rpcIndex + i) % endpoints.length;
        return JSON.parse(new TextDecoder().decode(new Uint8Array(bytes)));
      } catch (e) {
        lastErr = e;
      }
    }
    throw lastErr;
  }

  function describeRpcError(err) {
    const cause = err.cause || {};
    const info = cause.info || {};
    if (cause.name === "UNKNOWN_ACCOUNT" || /does not exist/.test(JSON.stringify(info))) {
      return "not-deployed";
    }
    return info.error_message || err.message || cause.name || "rpc error";
  }

  // ------------------------------------------------------- number format

  const ONE = 10n ** 18n;

  function big(x) {
    return BigInt(x || 0);
  }

  /** 123456789012345678901n -> "123.46" */
  function fmt(raw, decimals) {
    const d = decimals === undefined ? 2 : decimals;
    const v = big(raw);
    const whole = v / ONE;
    const frac = v % ONE;
    const fracStr = frac.toString().padStart(18, "0").slice(0, d);
    const wholeStr = whole.toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
    return d > 0 ? `${wholeStr}.${fracStr}` : wholeStr;
  }

  function fmtShort(raw) {
    const v = Number(big(raw) / ONE);
    if (v >= 1e9) return (v / 1e9).toFixed(2) + "B";
    if (v >= 1e6) return (v / 1e6).toFixed(2) + "M";
    if (v >= 1e3) return (v / 1e3).toFixed(1) + "k";
    return v.toFixed(0);
  }

  function pctOf(part, whole) {
    const w = big(whole);
    if (w === 0n) return "0%";
    return (Number((big(part) * 10000n) / w) / 100).toFixed(2) + "%";
  }

  function clock(ms) {
    if (ms < 0) ms = 0;
    const s = Math.floor(ms / 1000);
    if (s >= 86400) return Math.floor(s / 86400) + "d " + Math.floor((s % 86400) / 3600) + "h";
    if (s >= 3600)
      return (
        Math.floor(s / 3600) +
        ":" +
        String(Math.floor((s % 3600) / 60)).padStart(2, "0") +
        ":" +
        String(s % 60).padStart(2, "0")
      );
    return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
  }

  // ------------------------------------------------- the local clock

  /** Day index right now, from the config's genesis and the browser clock. */
  function localDay() {
    if (!state.config) return 0n;
    return M.dayOf(
      BigInt(Date.now()),
      BigInt(state.config.genesis_ms),
      BigInt(state.config.day_ms)
    );
  }

  function msIntoDay() {
    if (!state.config) return 0;
    const g = Number(state.config.genesis_ms);
    const d = Number(state.config.day_ms);
    const elapsed = Date.now() - g;
    return elapsed <= 0 ? 0 : elapsed % d;
  }

  /** Project an AccountView forward to right now, locally. */
  function projectBalance(view) {
    if (!view || !state.config) return 0n;
    const days = M.rotDays(
      big(view.settled_day),
      big(view.protected_until_day),
      localDay()
    );
    return M.applyRot(
      big(view.balance_settled),
      BigInt(state.config.rot_bps_per_day),
      days
    ).after;
  }

  function projectPending(view) {
    return big(view.balance_settled) - projectBalance(view);
  }

  // ---------------------------------------------------------- rendering

  function renderHeaderBits() {
    const d = CFG.deployments[state.key];
    $("contractId").textContent = state.contract;
    $("contractLink").href =
      (CFG.explorer[state.network] || CFG.explorer.testnet) +
      "/address/" +
      state.contract;
    if (d && d.blurb) $("contractLink").title = d.blurb;
  }

  function renderConfig() {
    const c = state.config;
    if (!c) return;
    const rot = (c.rot_bps_per_day / 100).toFixed(c.rot_bps_per_day % 100 ? 2 : 1) + "%";
    const cost = (c.stamp_cost_bps / 100).toFixed(2) + "%";
    $("pitchRot").textContent = rot;
    $("rRot").textContent = rot;
    $("rCost").textContent = cost;
    $("rDays").textContent = c.stamp_days;
  }

  function renderClock() {
    if (!state.config) return;
    const c = state.config;
    const day = localDay();
    const epochDays = BigInt(c.epoch_days);
    const epoch = M.epochOf(day, epochDays);

    $("cDay").textContent = day.toString();
    $("cEpoch").textContent = epoch.toString();

    const into = msIntoDay();
    const dayMs = Number(c.day_ms);
    $("cTick").textContent = clock(dayMs - into);
    $("cTickBar").style.width = ((into / dayMs) * 100).toFixed(1) + "%";

    const dayInEpoch = Number(day % epochDays);
    const epochMs = Number(epochDays) * dayMs;
    const intoEpoch = dayInEpoch * dayMs + into;
    $("cEpochIn").textContent = clock(epochMs - intoEpoch);
    $("cEpochBar").style.width = ((intoEpoch / epochMs) * 100).toFixed(1) + "%";
  }

  function renderStats() {
    const s = state.stats;
    const c = state.config;
    const el = $("stats");
    if (!s || !c) {
      el.innerHTML = '<div class="stat"><span class="k">waiting</span><span class="v">…</span></div>';
      return;
    }
    const cards = [
      ["total supply", fmtShort(s.total_supply) + " ROT", "can only shrink"],
      [
        "burned for good",
        fmtShort(s.burned) + " ROT",
        pctOf(s.burned, c.total_minted) + " of the launch supply",
      ],
      [
        "still in the faucet",
        fmtShort(s.faucet_remaining) + " ROT",
        "free, one share per account",
      ],
      ["in circulation", fmtShort(s.circulating) + " ROT", "claimed from the faucet"],
      [
        "compost waiting",
        fmtShort(s.composted_unclaimed) + " ROT",
        "rotted, not yet collected",
      ],
      [
        "rot owed, unpoked",
        fmtShort(s.unrealised_rot) + " ROT",
        "anyone can release this",
      ],
      ["holders", String(s.holders), "accounts registered"],
      [
        "longest streak",
        s.best_streak + (s.best_streak === 1 ? " epoch" : " epochs"),
        s.best_streak_holder || "nobody yet",
      ],
      [
        "stamped this epoch",
        fmtShort(s.current_epoch_weight) + " ROT",
        "weight sharing epoch " + s.epoch,
      ],
      [
        "locked in for next",
        fmtShort(s.next_epoch_weight) + " ROT",
        "weight for epoch " + (s.epoch + 1),
      ],
    ];
    el.innerHTML = cards
      .map(
        ([k, v, sub]) =>
          `<div class="stat"><span class="k">${esc(k)}</span><span class="v">${esc(
            v
          )}</span><div class="sub">${esc(sub)}</div></div>`
      )
      .join("");
  }

  function renderEpochs() {
    const el = $("epochs");
    if (!state.epochs.length) {
      el.innerHTML = '<div class="epoch"><div class="s">no epochs yet</div></div>';
      return;
    }
    el.innerHTML = state.epochs
      .slice()
      .reverse()
      .map((e) => {
        const paidPct = big(e.compost) > 0n ? pctOf(e.paid, e.compost) : "–";
        return `<div class="epoch ${esc(e.status)}">
          <div class="n">epoch ${e.epoch}</div>
          <div class="s">${esc(e.status)}</div>
          <div class="l"><span>weight</span><b>${fmtShort(e.weight)}</b></div>
          <div class="l"><span>compost</span><b>${fmtShort(e.compost)}</b></div>
          <div class="l"><span>paid out</span><b>${paidPct}</b></div>
        </div>`;
      })
      .join("");
  }

  function renderHolders() {
    const tbody = document.querySelector("#holders tbody");
    const onlyRotting = $("onlyRotting").checked;
    let rows = state.accounts.map((a) => ({
      id: a.account_id,
      balance: projectBalance(a),
      pending: projectPending(a),
      shield: big(a.protected_until_day) > localDay(),
      shieldLeft: Number(big(a.protected_until_day) - localDay()),
      streak: a.streak,
      claimable: big(a.claimable),
    }));
    if (onlyRotting) rows = rows.filter((r) => r.pending > 0n);
    rows.sort((a, b) => (b.balance > a.balance ? 1 : b.balance < a.balance ? -1 : 0));

    if (!rows.length) {
      tbody.innerHTML = '<tr><td colspan="7" class="empty">nobody here yet</td></tr>';
      return;
    }
    tbody.innerHTML = rows
      .map(
        (r, i) => `<tr>
          <td class="num">${i + 1}</td>
          <td class="acct"><a href="#" data-acct="${esc(r.id)}">${esc(r.id)}</a></td>
          <td class="num">${fmt(r.balance)}</td>
          <td class="num ${r.pending > 0n ? "bad" : ""}">${
            r.pending > 0n ? fmt(r.pending) : "–"
          }</td>
          <td>${
            r.shield
              ? `<span class="pill shielded">${r.shieldLeft}d left</span>`
              : '<span class="pill exposed">exposed</span>'
          }</td>
          <td class="num">${r.streak || "–"}</td>
          <td class="num ${r.claimable > 0n ? "good" : ""}">${
            r.claimable > 0n ? fmt(r.claimable) : "–"
          }</td>
        </tr>`
      )
      .join("");

    tbody.querySelectorAll("a[data-acct]").forEach((a) =>
      a.addEventListener("click", (e) => {
        e.preventDefault();
        $("accountInput").value = a.dataset.acct;
        lookupAccount();
      })
    );
  }

  function renderEvents() {
    const tbody = document.querySelector("#events tbody");
    if (!state.events.length) {
      tbody.innerHTML = '<tr><td colspan="5" class="empty">nothing has happened yet</td></tr>';
      return;
    }
    tbody.innerHTML = state.events
      .map(
        (e) => `<tr>
          <td class="num">${e.day}</td>
          <td><span class="tag ${esc(e.kind)}">${esc(e.kind)}</span></td>
          <td class="acct">${esc(e.account || "–")}</td>
          <td class="num">${fmt(e.amount)}</td>
          <td class="acct">${esc(e.note)}</td>
        </tr>`
      )
      .join("");
  }

  function renderAccount() {
    const a = state.account;
    if (!a) {
      $("accountCard").classList.add("hidden");
      $("accountEmpty").classList.remove("hidden");
      return;
    }
    $("accountEmpty").classList.add("hidden");
    $("accountCard").classList.remove("hidden");

    const bal = projectBalance(a);
    const pending = projectPending(a);
    const el = $("balBig");
    el.textContent = fmt(bal, 4);

    // Flash red the moment a day ticks over and the balance actually drops.
    if (state.lastBalance !== null && bal < state.lastBalance) {
      el.classList.add("dropping");
      setTimeout(() => el.classList.remove("dropping"), 700);
    }
    state.lastBalance = bal;

    const shielded = big(a.protected_until_day) > localDay();
    const pill = $("balShield");
    pill.textContent = shielded
      ? `shielded for ${Number(big(a.protected_until_day) - localDay())} more day(s)`
      : "exposed - rotting";
    pill.className = "pill " + (shielded ? "shielded" : "exposed");

    $("aSettled").textContent = fmt(a.balance_settled) + " ROT";
    $("aPending").textContent = (pending > 0n ? "-" : "") + fmt(pending) + " ROT";
    const perDay = M.applyRot(bal, BigInt(state.config.rot_bps_per_day), 1n).rotted;
    $("aPerDay").textContent = shielded ? "nothing, it is shielded" : "-" + fmt(perDay) + " ROT";
    $("aShield").textContent = shielded
      ? Number(big(a.protected_until_day) - localDay()) + " day(s)"
      : "none";
    $("aStreak").textContent =
      a.streak + " epoch(s), best " + a.best_streak;
    $("aClaim").textContent = fmt(a.claimable) + " ROT";

    const stamps = (a.stamps || [])
      .slice()
      .reverse()
      .map(
        (s) =>
          `<span>epoch ${s.epoch}: staked ${fmtShort(s.stake)}${
            big(s.claimed) > 0n ? ", collected " + fmtShort(s.claimed) : ""
          }</span>`
      )
      .join("");
    $("aStamps").innerHTML = stamps
      ? "<b>stakes:</b> " + stamps
      : '<span class="muted">no stamps yet - this account earns nothing from the compost</span>';

    renderCommands();
  }

  function esc(s) {
    return String(s == null ? "" : s).replace(
      /[&<>"']/g,
      (c) =>
        ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c])
    );
  }

  // ------------------------------------------------------------ commands

  function renderCommands() {
    const who = ($("accountInput").value || "YOUR-ACCOUNT.testnet").trim();
    const c = state.contract;
    const net = state.network;
    const call = (method, args, deposit) =>
      `near contract call-function as-transaction ${c} ${method} \\\n` +
      `  json-args '${args}' prepaid-gas '100.0 Tgas' \\\n` +
      `  attached-deposit '${deposit}' sign-as ${who} \\\n` +
      `  network-config ${net} sign-with-keychain send`;

    const rotting = state.accounts
      .filter((a) => projectPending(a) > 0n)
      .sort((a, b) => (projectPending(b) > projectPending(a) ? 1 : -1))
      .slice(0, 20)
      .map((a) => a.account_id);

    const cmds = [
      [
        "faucet_claim",
        "Take your one share. Registers you and pays the storage at the same time.",
        call("faucet_claim", "{}", "0.00125 NEAR"),
      ],
      [
        "stamp",
        "Burn 0.5% of your balance, shield the rest, and register for the next epoch's compost.",
        call("stamp", "{}", "0 NEAR"),
      ],
      [
        "claim",
        "Collect your cut of every finished epoch you stamped for.",
        call("claim", "{}", "0 NEAR"),
      ],
      [
        "poke",
        rotting.length
          ? `Force the rot out of ${rotting.length} sleeping account(s) and into this epoch's heap. If you are stamped this epoch, this pays you.`
          : "Settle other accounts. Nobody owes rot right now.",
        call(
          "poke",
          JSON.stringify({ account_ids: rotting.length ? rotting : [who] }),
          "0 NEAR"
        ),
      ],
      [
        "ft_balance_of",
        "A read, free and instant. Note it falls with the clock and no transaction.",
        `near contract call-function as-read-only ${c} ft_balance_of \\\n` +
          `  json-args '${JSON.stringify({ account_id: who })}' \\\n` +
          `  network-config ${net} now`,
      ],
      [
        "audit_supply",
        "Ask the contract whether its own books still balance. It should always say true.",
        `near contract call-function as-read-only ${c} audit_supply \\\n` +
          `  json-args '{}' network-config ${net} now`,
      ],
    ];

    $("cmds").innerHTML = cmds
      .map(
        ([n, d, cmd], i) => `<div class="cmd">
          <div class="t">
            <div><span class="n">${esc(n)}</span> <span class="d">${esc(d)}</span></div>
            <button class="ghost" data-copy="${i}">copy</button>
          </div>
          <pre id="cmd${i}">${esc(cmd)}</pre>
        </div>`
      )
      .join("");

    $("cmds")
      .querySelectorAll("button[data-copy]")
      .forEach((b) =>
        b.addEventListener("click", () => {
          copy($("cmd" + b.dataset.copy).textContent, b);
        })
      );
  }

  function copy(text, btn) {
    const done = () => {
      const old = btn.textContent;
      btn.textContent = "copied";
      setTimeout(() => (btn.textContent = old), 1200);
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(done, () => fallback(text, done));
    } else {
      fallback(text, done);
    }
  }

  function fallback(text, done) {
    const t = document.createElement("textarea");
    t.value = text;
    t.style.position = "fixed";
    t.style.opacity = "0";
    document.body.appendChild(t);
    t.select();
    try {
      document.execCommand("copy");
      done();
    } catch (e) {
      /* nothing we can do */
    }
    document.body.removeChild(t);
  }

  // ----------------------------------------------------------- playground

  function chart(title, rows, series, yLabel) {
    const W = 760,
      H = 290,
      PL = 64,
      PR = 150,
      PT = 26,
      PB = 32;
    const xs = rows.map((r) => r.day);
    const xMin = Math.min(...xs),
      xMax = Math.max(...xs);
    const vals = series.flatMap(([k]) => rows.map((r) => r[k]));
    const yMax = Math.max(...vals, 1) * 1.08;
    const px = (d) => PL + ((d - xMin) / Math.max(1, xMax - xMin)) * (W - PL - PR);
    const py = (v) => H - PB - (v / yMax) * (H - PT - PB);
    const fm = (v) =>
      v >= 1e6
        ? (v / 1e6).toFixed(1) + "M"
        : v >= 1e3
        ? (v / 1e3).toFixed(1) + "k"
        : v.toFixed(0);

    let s = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${W} ${H}" font-family="IBM Plex Mono, ui-monospace, monospace" font-size="10">`;
    s += `<rect width="${W}" height="${H}" fill="#0b0b0a"/>`;
    s += `<text x="${PL}" y="17" fill="#e7e5e4" font-size="11">${esc(title)}</text>`;
    for (let k = 0; k <= 4; k++) {
      const v = (yMax * k) / 4,
        y = py(v);
      s += `<line x1="${PL}" y1="${y.toFixed(1)}" x2="${W - PR}" y2="${y.toFixed(
        1
      )}" stroke="#27272a"/>`;
      s += `<text x="${PL - 6}" y="${(y + 3).toFixed(
        1
      )}" fill="#71717a" text-anchor="end">${fm(v)}</text>`;
    }
    const epochDays = state.config ? Number(state.config.epoch_days) : 7;
    for (let d = xMin; d <= xMax; d += epochDays) {
      const x = px(d);
      s += `<line x1="${x.toFixed(1)}" y1="${PT}" x2="${x.toFixed(1)}" y2="${
        H - PB
      }" stroke="#1c1917"/>`;
      s += `<text x="${x.toFixed(1)}" y="${
        H - PB + 13
      }" fill="#52525b" text-anchor="middle">d${d}</text>`;
    }
    series.forEach(([k, label, color], i) => {
      const pts = rows.map((r) => `${px(r.day).toFixed(1)},${py(r[k]).toFixed(1)}`).join(" ");
      s += `<polyline points="${pts}" fill="none" stroke="${color}" stroke-width="1.8" stroke-linejoin="round"/>`;
      const ly = PT + 12 + i * 14;
      s += `<line x1="${W - PR + 8}" y1="${ly - 3}" x2="${
        W - PR + 24
      }" y2="${ly - 3}" stroke="${color}" stroke-width="2"/>`;
      s += `<text x="${W - PR + 28}" y="${ly}" fill="#a1a1aa">${esc(label)}</text>`;
    });
    if (yLabel) s += `<text x="${PL}" y="${H - 5}" fill="#52525b">${esc(yLabel)}</text>`;
    return s + "</svg>";
  }

  function runPlayground() {
    const rotBps = +$("pRot").value;
    const costBps = +$("pCost").value;
    const part = +$("pPart").value / 100;
    const days = +$("pDays").value;
    const whale = +$("pWhale").value;

    $("pvRot").textContent = (rotBps / 100).toFixed(1) + "%";
    $("pvCost").textContent = (costBps / 100).toFixed(2) + "%";
    $("pvPart").textContent = Math.round(part * 100) + "%";
    $("pvDays").textContent = days + (days === 1 ? " day" : " days");
    $("pvWhale").textContent = whale + "x";

    const out = M.simulate({
      days: 60,
      holders: 10,
      participation: part,
      rotBps,
      stampCostBps: costBps,
      stampDays: days,
      epochDays: 7,
      whale,
    });

    const sum = out.summary;
    const sign = (v) => (v >= 0 ? "+" : "") + v.toFixed(1) + "%";
    const cls = (v) => (v >= 0 ? "good" : "bad");
    $("simSummary").innerHTML = `
      <div><span class="k">supply after 60 days</span><span class="v ${cls(
        sum.supplyChangePct
      )}">${sign(sum.supplyChangePct)}</span></div>
      <div><span class="k">burned</span><span class="v">${sum.burned.toFixed(
        0
      )} ROT</span></div>
      <div><span class="k">a stamper's balance</span><span class="v ${cls(
        sum.stamperChangePct
      )}">${sum.nStampers ? sign(sum.stamperChangePct) : "no one"}</span></div>
      <div><span class="k">a sleeper's balance</span><span class="v ${cls(
        sum.sleeperChangePct
      )}">${sum.nSleepers ? sign(sum.sleeperChangePct) : "no one"}</span></div>`;

    $("chartSupply").innerHTML = chart(
      "supply over 60 days",
      out.rows,
      [
        ["totalSupply", "total supply", "#16a34a"],
        ["burned", "burned", "#dc2626"],
        ["composted", "compost waiting", "#2563eb"],
      ],
      "ROT"
    );
    $("chartGroups").innerHTML = chart(
      "average balance: stampers against sleepers",
      out.rows,
      [
        ["avgStamper", "a stamper", "#16a34a"],
        ["avgSleeper", "a sleeper", "#dc2626"],
      ],
      "ROT"
    );
  }

  // ---------------------------------------------------------------- data

  async function lookupAccount() {
    const id = ($("accountInput").value || "").trim().toLowerCase();
    state.accountId = id;
    if (!id) {
      state.account = null;
      renderAccount();
      return;
    }
    try {
      const v = await rpc("get_account", { account_id: id });
      state.account = v;
      if (!v) {
        $("accountEmpty").classList.remove("hidden");
        $("accountEmpty").innerHTML =
          esc(id) +
          " holds no $ROT yet. Run <code>faucet_claim</code> below to register and take a share.";
        $("accountCard").classList.add("hidden");
        return;
      }
      localStorage.setItem("rot.account", id);
    } catch (e) {
      state.account = null;
      $("accountEmpty").classList.remove("hidden");
      $("accountEmpty").textContent = "could not read that account: " + e.message;
    }
    renderAccount();
  }

  async function refresh() {
    try {
      const [config, stats] = await Promise.all([
        state.config ? Promise.resolve(state.config) : rpc("get_config", {}),
        rpc("get_stats", {}),
      ]);
      state.config = config;
      state.stats = stats;
      setRpcState(true, "live");
      renderConfig();

      const [epochs, events, accounts] = await Promise.all([
        rpc("get_epochs", { limit: 10 }),
        rpc("get_events", { limit: 40 }),
        rpc("list_accounts", { limit: 100 }),
      ]);
      state.epochs = epochs || [];
      state.events = events || [];
      state.accounts = accounts || [];

      renderStats();
      renderEpochs();
      renderHolders();
      renderEvents();
      if (state.accountId) await lookupAccount();
      renderCommands();
    } catch (e) {
      const notDeployed = /not-deployed/.test(e.message);
      setRpcState(false, notDeployed ? "not deployed yet" : "rpc: " + e.message);
      if (notDeployed) {
        $("stats").innerHTML =
          '<div class="stat"><span class="k">contract</span><span class="v">not deployed yet</span>' +
          '<div class="sub">Pick another deployment above, or pass ?contract=… in the URL. ' +
          "Everything below still works: the fast-forward runs entirely in your browser.</div></div>";
      }
    }
  }

  function setRpcState(ok, text) {
    state.live = ok;
    const el = $("rpcState");
    el.textContent = text;
    el.className = "rpc " + (ok ? "live" : "dead");
  }

  // ---------------------------------------------------------------- init

  function initDeployments() {
    const params = new URLSearchParams(location.search);
    const sel = $("deployment");
    Object.entries(CFG.deployments).forEach(([key, d]) => {
      const o = document.createElement("option");
      o.value = key;
      o.textContent = d.label;
      sel.appendChild(o);
    });

    const override = params.get("contract");
    if (override) {
      const o = document.createElement("option");
      o.value = "custom";
      o.textContent = "custom - " + override;
      sel.appendChild(o);
      CFG.deployments.custom = {
        label: "custom",
        contract: override,
        network: params.get("network") || "testnet",
      };
      state.key = "custom";
    } else {
      state.key = params.get("deployment") || CFG.default;
    }

    sel.value = state.key;
    applyDeployment();
    sel.addEventListener("change", () => {
      state.key = sel.value;
      state.config = null;
      state.stats = null;
      state.lastBalance = null;
      applyDeployment();
      refresh();
    });
  }

  function applyDeployment() {
    const d = CFG.deployments[state.key] || CFG.deployments[CFG.default];
    state.contract = d.contract;
    state.network = d.network || "testnet";
    renderHeaderBits();
    setRpcState(false, "connecting…");
  }

  function initMathBadge() {
    const r = M.selfTest();
    const el = $("mathBadge");
    if (r.ok) {
      el.className = "badge ok";
      el.textContent = `math matches the contract (${r.checked} vectors)`;
      el.title =
        "This page's model is a mirror of src/math.rs. These same vectors are asserted by cargo test and by the Python simulation.";
    } else {
      el.className = "badge fail";
      el.textContent = "math mismatch: " + r.error;
    }
  }

  function init() {
    initMathBadge();
    initDeployments();

    $("accountGo").addEventListener("click", lookupAccount);
    $("accountInput").addEventListener("keydown", (e) => {
      if (e.key === "Enter") lookupAccount();
    });
    const saved = localStorage.getItem("rot.account");
    if (saved) {
      $("accountInput").value = saved;
      state.accountId = saved;
    }

    $("onlyRotting").addEventListener("change", renderHolders);
    $("copyPoke").addEventListener("click", (e) => {
      const ids = state.accounts
        .filter((a) => projectPending(a) > 0n)
        .sort((a, b) => (projectPending(b) > projectPending(a) ? 1 : -1))
        .slice(0, 20)
        .map((a) => a.account_id);
      const who = ($("accountInput").value || "YOUR-ACCOUNT.testnet").trim();
      copy(
        `near contract call-function as-transaction ${state.contract} poke \\\n` +
          `  json-args '${JSON.stringify({ account_ids: ids })}' \\\n` +
          `  prepaid-gas '300.0 Tgas' attached-deposit '0 NEAR' sign-as ${who} \\\n` +
          `  network-config ${state.network} sign-with-keychain send`,
        e.target
      );
    });

    ["pRot", "pCost", "pPart", "pDays", "pWhale"].forEach((id) =>
      $(id).addEventListener("input", runPlayground)
    );
    $("pReset").addEventListener("click", () => {
      $("pRot").value = 200;
      $("pCost").value = 50;
      $("pPart").value = 50;
      $("pDays").value = 7;
      $("pWhale").value = 0;
      runPlayground();
    });

    runPlayground();
    renderCommands();
    refresh();

    // The chain is polled every 6s; the clock and the balances are recomputed
    // locally twice a second so the rot is visible as it happens.
    setInterval(refresh, 6000);
    setInterval(() => {
      renderClock();
      if (state.account) renderAccount();
      if (state.accounts.length) renderHolders();
    }, 500);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
