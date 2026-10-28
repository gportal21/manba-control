// Ventana de configuración: edita los perfiles y muestra el mando en vivo.
const { invoke } = window.__TAURI__.core;

const LIVE_INTERVAL_MS = 33;
const MOUSE_ACTIONS = [
  ["raton:izquierdo", "Clic izquierdo"],
  ["raton:derecho", "Clic derecho"],
  ["raton:medio", "Clic central"],
];
const REASON_GAME = 1;
const REASON_SHORTCUT = 2;
const TURBO_RATES = [5, 10, 15, 20, 30];

let options = null;
let currentName = null;
let saved = null;
let profile = null;

const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const clone = (value) => JSON.parse(JSON.stringify(value));

function getPath(object, path) {
  return path.split(".").reduce((value, key) => value[key], object);
}

function setPath(object, path, value) {
  const keys = path.split(".");
  const last = keys.pop();
  keys.reduce((value, key) => value[key], object)[last] = value;
}

function formatValue(path, value) {
  if (path.endsWith("intervalo_ms")) return `${value} ms`;
  if (path.endsWith("velocidad_raton")) return `${Math.round(value)} px/s`;
  if (path.endsWith("umbral")) return `${value}`;
  if (path.endsWith("intensidad")) return `${Math.round(value * 100)} %`;
  return Number(value).toFixed(2);
}

function readInput(input) {
  if (input.type === "checkbox") return input.checked;
  if (input.type === "range" || input.type === "number") return Number(input.value);
  return input.value;
}

function writeInput(input, value) {
  if (input.type === "checkbox") input.checked = value;
  else input.value = value;
}

function fillSelect(select, names, selected) {
  select.replaceChildren(
    ...names.map((name) => {
      const option = document.createElement("option");
      option.value = name;
      option.textContent = name;
      return option;
    }),
  );
  select.value = selected;
}

// --- Formulario -------------------------------------------------------------

function buildSticks() {
  const template = $("#stick-fields");
  for (const card of $$("[data-stick]")) {
    const stick = card.dataset.stick;
    const fields = $(".fields", card);
    fields.append(template.content.cloneNode(true));
    for (const input of $$("[data-key]", fields)) {
      input.dataset.path = `${stick}.${input.dataset.key}`;
    }
    for (const out of $$("[data-out-key]", fields)) {
      out.dataset.out = `${stick}.${out.dataset.outKey}`;
    }
  }
}

function actionOptions(self) {
  const select = document.createElement("select");
  const add = (parent, value, label) => {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    parent.append(option);
  };
  const group = (label) => {
    const optgroup = document.createElement("optgroup");
    optgroup.label = label;
    select.append(optgroup);
    return optgroup;
  };

  add(select, "", "Sin cambios");
  add(select, "ninguno", "Desactivado");
  const pad = group("Otro control del mando");
  for (const control of options.controls) {
    if (control.name !== self) add(pad, control.name, control.name);
  }
  const mouse = group("Ratón");
  for (const [value, label] of MOUSE_ACTIONS) add(mouse, value, label);
  const keyboard = group("Teclado");
  for (const key of options.keys) add(keyboard, `tecla:${key}`, key);
  return select;
}

// El perfil puede escribirse a mano con otras mayúsculas: se busca la opción
// equivalente y, si no existe, se añade tal cual para no perder el valor.
function selectAction(select, value) {
  if (!value) {
    select.value = "";
    return;
  }
  const match = [...select.options].find((o) => o.value.toLowerCase() === value.toLowerCase());
  if (match) {
    select.value = match.value;
  } else {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = value;
    select.append(option);
    select.value = value;
  }
}

function findKey(object, name) {
  return Object.keys(object).find((k) => k.toLowerCase() === name.toLowerCase());
}

function turboSelect(control) {
  const select = document.createElement("select");
  select.className = "turbo";
  select.setAttribute("aria-label", `Turbo de ${control}`);
  for (const [value, label] of [["", "Sin turbo"], ...TURBO_RATES.map((r) => [String(r), `Turbo ${r}/s`])]) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = label;
    select.append(option);
  }
  select.addEventListener("change", () => {
    const key = findKey(profile.turbo, control);
    if (key) delete profile.turbo[key];
    if (select.value) profile.turbo[control] = Number(select.value);
    onChange();
  });
  return select;
}

function setTurbo(select, rate) {
  const value = rate ? String(rate) : "";
  if (value && ![...select.options].some((o) => o.value === value)) {
    const option = document.createElement("option");
    option.value = value;
    option.textContent = `Turbo ${value}/s`;
    select.append(option);
  }
  select.value = value;
}

// Las macros del perfil aparecen como acciones en todas las filas.
function refreshMacroOptions() {
  const names = Object.keys(profile.macros);
  for (const row of $$(".button-row")) {
    const select = $("select:not(.turbo)", row);
    $("optgroup[data-macros]", select)?.remove();
    if (names.length) {
      const group = document.createElement("optgroup");
      group.label = "Macros";
      group.dataset.macros = "";
      for (const name of names) {
        const option = document.createElement("option");
        option.value = `macro:${name}`;
        option.textContent = `Macro: ${name}`;
        group.append(option);
      }
      select.insertBefore(group, select.children[2]);
    }
    const key = findKey(profile.botones, row.dataset.control);
    selectAction(select, key ? profile.botones[key] : "");
  }
}

function buildButtons() {
  const container = $("#buttons");
  for (const control of options.controls) {
    const row = document.createElement("div");
    row.className = "button-row";
    row.dataset.control = control.name;

    const dot = document.createElement("span");
    dot.className = "dot";
    const name = document.createElement("span");
    name.className = "name";
    name.textContent = control.name;
    const select = actionOptions(control.name);
    select.addEventListener("change", () => {
      const value = select.value;
      // Asignar un control a sí mismo equivale a no cambiarlo.
      if (value === "" || value.toLowerCase() === control.name.toLowerCase()) {
        delete profile.botones[control.name];
      } else {
        profile.botones[control.name] = value;
      }
      onChange();
    });

    row.append(dot, name, select, turboSelect(control.name));
    container.append(row);
  }
}

function bindInputs() {
  for (const input of $$("[data-path]")) {
    input.addEventListener("input", () => {
      setPath(profile, input.dataset.path, readInput(input));
      onChange();
    });
  }
}

function fillForm() {
  for (const input of $$("[data-path]")) {
    writeInput(input, getPath(profile, input.dataset.path));
  }
  for (const row of $$(".button-row")) {
    const name = row.dataset.control;
    const turboKey = findKey(profile.turbo, name);
    setTurbo($(".turbo", row), turboKey ? profile.turbo[turboKey] : null);
  }
  refreshMacroOptions();
  renderGames();
  renderMacros();
  refreshLabels();
}

function refreshLabels() {
  for (const out of $$("[data-out]")) {
    out.textContent = formatValue(out.dataset.out, getPath(profile, out.dataset.out));
  }
  for (const card of $$("[data-stick]")) {
    const mode = profile[card.dataset.stick].modo;
    $(".mouse-only", card).hidden = mode !== "raton";
  }
  for (const row of $$(".button-row")) {
    row.classList.toggle("changed", $("select:not(.turbo)", row).value !== "");
    const turbo = $(".turbo", row);
    turbo.classList.toggle("on", turbo.value !== "");
  }
}

function isDirty() {
  return JSON.stringify(profile) !== JSON.stringify(saved);
}

function setSaveStatus(text, kind = "") {
  const status = $("#save-status");
  status.textContent = text;
  status.className = `save-status ${kind}`;
}

function onChange() {
  refreshLabels();
  const dirty = isDirty();
  $("#save").disabled = !dirty;
  $("#discard").disabled = !dirty;
  setSaveStatus(dirty ? `Cambios sin guardar en "${currentName}"` : "Sin cambios", dirty ? "dirty" : "");
}

async function save() {
  try {
    await invoke("save_profile", { name: currentName, profile });
    saved = clone(profile);
    onChange();
    setSaveStatus("Guardado. El servicio ya aplica los cambios.");
  } catch (error) {
    setSaveStatus(`No se pudo guardar: ${error}`, "error");
  }
}

function discard() {
  profile = clone(saved);
  fillForm();
  onChange();
}

// --- Macros -----------------------------------------------------------------

function stepText(step) {
  return step.pulsar.join(" + ");
}

function parseStepText(text) {
  return text.split("+").map((part) => part.trim()).filter(Boolean);
}

function uniqueMacroName(base) {
  let name = base;
  for (let n = 2; findKey(profile.macros, name); n++) name = `${base} ${n}`;
  return name;
}

// Cambiar el nombre actualiza también los botones que lanzaban la macro.
function renameMacro(oldName, newName) {
  newName = newName.trim();
  if (!newName || newName === oldName) return false;
  if (findKey(profile.macros, newName)) return false;
  const entries = Object.entries(profile.macros).map(([k, v]) => [k === oldName ? newName : k, v]);
  profile.macros = Object.fromEntries(entries);
  for (const [control, action] of Object.entries(profile.botones)) {
    if (action.toLowerCase() === `macro:${oldName}`.toLowerCase()) profile.botones[control] = `macro:${newName}`;
  }
  return true;
}

function renderMacros() {
  const container = $("#macros");
  container.replaceChildren(
    ...Object.entries(profile.macros).map(([name, macro]) => {
      const box = document.createElement("div");
      box.className = "macro";

      const head = document.createElement("div");
      head.className = "macro-head";
      const nameInput = document.createElement("input");
      nameInput.type = "text";
      nameInput.value = name;
      nameInput.maxLength = 32;
      nameInput.setAttribute("aria-label", "Nombre de la macro");
      nameInput.addEventListener("change", () => {
        if (renameMacro(name, nameInput.value)) {
          renderMacros();
          refreshMacroOptions();
          onChange();
        } else {
          nameInput.value = name;
        }
      });
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "secondary danger";
      remove.textContent = "Eliminar macro";
      remove.addEventListener("click", () => {
        delete profile.macros[name];
        // Los botones que la lanzaban vuelven a su función normal.
        for (const [control, action] of Object.entries(profile.botones)) {
          if (action.toLowerCase() === `macro:${name}`.toLowerCase()) delete profile.botones[control];
        }
        renderMacros();
        refreshMacroOptions();
        onChange();
      });
      head.append(nameInput, remove);
      box.append(head);

      macro.pasos.forEach((step, index) => {
        const row = document.createElement("div");
        row.className = "macro-step";
        const number = document.createElement("span");
        number.className = "index";
        number.textContent = index + 1;
        const actions = document.createElement("input");
        actions.type = "text";
        actions.value = stepText(step);
        actions.placeholder = "Espera (sin acciones)";
        actions.spellcheck = false;
        actions.setAttribute("aria-label", `Acciones del paso ${index + 1}`);
        actions.addEventListener("input", () => {
          step.pulsar = parseStepText(actions.value);
          onChange();
        });
        const ms = document.createElement("input");
        ms.type = "number";
        ms.min = "1";
        ms.max = "5000";
        ms.value = step.ms;
        ms.title = "Duración en milisegundos";
        ms.setAttribute("aria-label", `Duración del paso ${index + 1} en milisegundos`);
        ms.addEventListener("input", () => {
          step.ms = Math.round(Number(ms.value)) || 0;
          onChange();
        });
        const drop = document.createElement("button");
        drop.type = "button";
        drop.className = "icon-button";
        drop.textContent = "×";
        drop.setAttribute("aria-label", `Quitar el paso ${index + 1}`);
        drop.disabled = macro.pasos.length === 1;
        drop.addEventListener("click", () => {
          macro.pasos.splice(index, 1);
          renderMacros();
          onChange();
        });
        row.append(number, actions, ms, drop);
        box.append(row);
      });

      const add = document.createElement("button");
      add.type = "button";
      add.className = "secondary";
      add.textContent = "Añadir paso";
      add.addEventListener("click", () => {
        macro.pasos.push({ pulsar: [], ms: 50 });
        renderMacros();
        onChange();
      });
      box.append(add);
      return box;
    }),
  );
}

function bindMacros() {
  $("#macro-new").addEventListener("click", () => {
    const name = uniqueMacroName("Macro");
    profile.macros[name] = { pasos: [{ pulsar: ["A"], ms: 60 }] };
    renderMacros();
    refreshMacroOptions();
    onChange();
  });
}

// --- Juegos -----------------------------------------------------------------

function renderGames() {
  const container = $("#games");
  container.replaceChildren(
    ...profile.juegos.map((game, index) => {
      const chip = document.createElement("span");
      chip.className = "chip";
      chip.textContent = game;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "×";
      remove.setAttribute("aria-label", `Quitar ${game}`);
      remove.addEventListener("click", () => {
        profile.juegos.splice(index, 1);
        renderGames();
        onChange();
      });
      chip.append(remove);
      return chip;
    }),
  );
}

function bindGames() {
  const input = $("#game-input");
  input.addEventListener("focus", async () => {
    const apps = await invoke("running_apps");
    $("#running-apps").replaceChildren(
      ...apps.map((app) => {
        const option = document.createElement("option");
        option.value = app;
        return option;
      }),
    );
  });
  $("#game-form").addEventListener("submit", (event) => {
    event.preventDefault();
    // Solo el nombre del ejecutable, aunque se pegue una ruta completa.
    let game = input.value.trim().split(/[\\/]/).pop();
    if (!game) return;
    if (!game.toLowerCase().endsWith(".exe")) game += ".exe";
    if (!profile.juegos.some((g) => g.toLowerCase() === game.toLowerCase())) {
      profile.juegos.push(game);
      renderGames();
      onChange();
    }
    input.value = "";
  });
}

// --- Perfiles ---------------------------------------------------------------

let nameMode = null;

async function refreshProfiles(select) {
  const info = await invoke("profiles");
  fillSelect($("#profile-select"), info.names, select ?? currentName);
  fillSelect($("#base-select"), info.names, info.base);
  $("#shortcuts").checked = info.shortcuts;
  return info;
}

async function openProfile(name) {
  try {
    saved = await invoke("load_profile", { name });
  } catch (error) {
    // Un perfil con errores se abre con valores por defecto para poder arreglarlo.
    setSaveStatus(`El perfil "${name}" tenía errores (${error}); se muestran valores por defecto.`, "error");
    saved = defaultProfile();
  }
  currentName = name;
  profile = clone(saved);
  $("#profile-select").value = name;
  const isDefault = name === options.default_profile;
  $("#profile-rename").disabled = isDefault;
  $("#profile-delete").disabled = isDefault;
  fillForm();
  onChange();
}

function showNameForm(mode) {
  nameMode = mode;
  const labels = {
    new: "Nombre del perfil nuevo",
    duplicate: `Nombre de la copia de "${currentName}"`,
    rename: `Nuevo nombre para "${currentName}"`,
  };
  $("#name-label").textContent = labels[mode];
  $("#name-input").value = mode === "rename" ? currentName : mode === "duplicate" ? `${currentName} (copia)` : "";
  $("#name-error").hidden = true;
  $("#name-form").hidden = false;
  $("#name-input").focus();
  $("#name-input").select();
}

function hideNameForm() {
  nameMode = null;
  $("#name-form").hidden = true;
}

// Cambiar de perfil con cambios pendientes los perdería: se pide guardar antes.
function blockedByChanges() {
  if (!isDirty()) return false;
  setSaveStatus("Guarda o descarta los cambios antes de cambiar de perfil.", "error");
  return true;
}

function bindProfiles() {
  $("#profile-select").addEventListener("change", (event) => {
    const name = event.target.value;
    if (blockedByChanges()) {
      event.target.value = currentName;
      return;
    }
    hideNameForm();
    openProfile(name);
  });

  $("#profile-new").addEventListener("click", () => !blockedByChanges() && showNameForm("new"));
  $("#profile-duplicate").addEventListener("click", () => !blockedByChanges() && showNameForm("duplicate"));
  $("#profile-rename").addEventListener("click", () => !blockedByChanges() && showNameForm("rename"));
  $("#name-cancel").addEventListener("click", hideNameForm);

  $("#name-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const name = $("#name-input").value;
    try {
      let result;
      if (nameMode === "rename") {
        result = await invoke("rename_profile", { old: currentName, new: name });
      } else {
        const from = nameMode === "duplicate" ? currentName : null;
        result = await invoke("create_profile", { name, from });
      }
      hideNameForm();
      await refreshProfiles(result);
      await openProfile(result);
    } catch (error) {
      $("#name-error").textContent = String(error);
      $("#name-error").hidden = false;
    }
  });

  // Borrar pide un segundo clic para confirmar.
  const remove = $("#profile-delete");
  let confirmTimer = null;
  remove.addEventListener("click", async () => {
    if (!confirmTimer) {
      remove.textContent = "Pulsa otra vez para borrar";
      confirmTimer = setTimeout(() => {
        remove.textContent = "Eliminar";
        confirmTimer = null;
      }, 4000);
      return;
    }
    clearTimeout(confirmTimer);
    confirmTimer = null;
    remove.textContent = "Eliminar";
    try {
      await invoke("delete_profile", { name: currentName });
      saved = profile;
      await refreshProfiles(options.default_profile);
      await openProfile(options.default_profile);
    } catch (error) {
      setSaveStatus(`No se pudo borrar: ${error}`, "error");
    }
  });
}

// --- General ----------------------------------------------------------------

function showGeneralError(error) {
  const box = $("#general-error");
  box.textContent = error ? String(error) : "";
  box.hidden = !error;
}

async function bindGeneral() {
  $("#base-select").addEventListener("change", async (event) => {
    try {
      await invoke("set_base", { name: event.target.value });
      showGeneralError(null);
    } catch (error) {
      showGeneralError(error);
    }
  });

  $("#shortcuts").addEventListener("change", async (event) => {
    try {
      await invoke("set_shortcuts", { enabled: event.target.checked });
      showGeneralError(null);
    } catch (error) {
      showGeneralError(error);
    }
  });

  const autostart = $("#autostart");
  autostart.checked = await invoke("autostart_enabled");
  autostart.addEventListener("change", async () => {
    try {
      autostart.checked = await invoke("set_autostart", { enabled: autostart.checked });
      showGeneralError(null);
    } catch (error) {
      autostart.checked = await invoke("autostart_enabled");
      showGeneralError(error);
    }
  });
}

// --- Estado en vivo ---------------------------------------------------------

function cssVar(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function drawStick(canvas, config, raw, processed) {
  const ratio = window.devicePixelRatio || 1;
  const size = canvas.clientWidth;
  if (canvas.width !== size * ratio) {
    canvas.width = size * ratio;
    canvas.height = size * ratio;
  }
  const ctx = canvas.getContext("2d");
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, size, size);

  const c = size / 2;
  const r = c - 8;
  const border = cssVar("--border");
  const accent = cssVar("--accent");

  // Recorrido completo.
  ctx.fillStyle = cssVar("--surface-2");
  ctx.beginPath();
  ctx.arc(c, c, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.strokeStyle = border;
  ctx.lineWidth = 1;
  ctx.stroke();

  // Ejes.
  ctx.beginPath();
  ctx.moveTo(c - r, c);
  ctx.lineTo(c + r, c);
  ctx.moveTo(c, c - r);
  ctx.lineTo(c, c + r);
  ctx.stroke();

  // Zona exterior.
  ctx.setLineDash([4, 4]);
  ctx.strokeStyle = cssVar("--muted");
  ctx.beginPath();
  ctx.arc(c, c, r * config.zona_exterior, 0, Math.PI * 2);
  ctx.stroke();
  ctx.setLineDash([]);

  // Zona muerta.
  ctx.fillStyle = cssVar("--accent-soft");
  ctx.beginPath();
  ctx.arc(c, c, r * config.zona_muerta, 0, Math.PI * 2);
  ctx.fill();

  if (raw) {
    // Posición física del stick.
    ctx.fillStyle = cssVar("--raw");
    ctx.beginPath();
    ctx.arc(c + (raw[0] / 32767) * r, c - (raw[1] / 32767) * r, 5, 0, Math.PI * 2);
    ctx.fill();
  }
  if (processed) {
    // Lo que recibe el juego con este perfil.
    ctx.fillStyle = accent;
    ctx.beginPath();
    ctx.arc(c + processed[0] * r, c - processed[1] * r, 6, 0, Math.PI * 2);
    ctx.fill();
  }
}

function setPill(id, kind, text) {
  const pill = $(id);
  pill.className = `pill ${kind}`;
  $("b", pill).textContent = text;
}

function showNotice(text, kind = "") {
  const notice = $("#notice");
  notice.hidden = !text;
  notice.textContent = text || "";
  notice.className = `notice ${kind}`;
}

function updateActiveLine(view) {
  const line = $("#active-line");
  if (!view.service) {
    line.textContent = "El servicio no está en marcha.";
    return;
  }
  const reason = {
    [REASON_GAME]: "porque el juego está en primer plano",
    [REASON_SHORTCUT]: "elegido con el mando",
  }[view.state.reason] ?? "es el perfil base";
  const paused = view.state.enabled ? "" : " La reasignación está en pausa.";
  const name = document.createElement("b");
  name.textContent = view.active_profile;
  line.replaceChildren("Ahora el servicio usa ", name, ` (${reason}).${paused}`);
}

function updateLive(view) {
  const state = view.state;

  if (!view.service) {
    setPill("#pill-service", "bad", "Servicio detenido");
    setPill("#pill-pad", "", "Mando");
    setPill("#pill-virtual", "", "Mando virtual");
  } else {
    setPill("#pill-service", "ok", state.enabled ? "Servicio activo" : "Servicio (en pausa)");
    setPill("#pill-pad", state.connected ? "ok" : "warn", state.connected ? "Mando conectado" : "Sin mando");
    const virtual = [
      ["", "Mando virtual"],
      ["ok", "Mando virtual activo"],
      ["bad", "Falta ViGEmBus"],
      ["bad", "Error del mando virtual"],
    ][state.virtual_pad] || ["", "Mando virtual"];
    setPill("#pill-virtual", virtual[0], virtual[1]);
  }
  updateActiveLine(view);

  if (view.error) {
    showNotice(`Este perfil tiene un error y no se puede guardar: ${view.error}`, "error");
  } else if (!view.service) {
    showNotice("El servicio no está en marcha. Puedes editar los perfiles, pero no verás el mando en vivo ni se aplicarán los cambios hasta que lo inicies.");
  } else if (!state.profile_ok) {
    showNotice("Algún perfil guardado tiene errores y el servicio no lo puede usar. Ábrelo aquí y guárdalo para corregirlo.", "error");
  } else {
    showNotice("");
  }

  // La vista en vivo se dibuja con el perfil que se está editando.
  const live = view.service && state.connected ? state : null;
  for (const [card, stick, raw, processed] of [
    [$('[data-stick="stick_izquierdo"]'), "stick_izquierdo", live?.raw_left, view.left],
    [$('[data-stick="stick_derecho"]'), "stick_derecho", live?.raw_right, view.right],
  ]) {
    drawStick($("canvas", card), profile[stick], raw, processed);
    $(".readout", card).textContent = raw && processed
      ? `Físico (${raw[0]}, ${raw[1]})  ->  juego (${processed[0].toFixed(2)}, ${processed[1].toFixed(2)})`
      : "Sin datos del mando";
  }

  const threshold = profile.gatillos.umbral;
  for (const [side, value] of [["lt", live?.raw_lt ?? 0], ["rt", live?.raw_rt ?? 0]]) {
    $(`#bar-${side}`).style.width = `${(value / 255) * 100}%`;
    $(`#mark-${side}`).style.left = `${(threshold / 255) * 100}%`;
    $(`#val-${side}`).textContent = value;
  }

  for (const row of $$(".button-row")) {
    const control = options.controls.find((c) => c.name === row.dataset.control);
    let pressed = false;
    if (live) {
      if (control.trigger === "LT") pressed = live.raw_lt > threshold;
      else if (control.trigger === "RT") pressed = live.raw_rt > threshold;
      else pressed = (live.raw_buttons & control.mask) !== 0;
    }
    $(".dot", row).classList.toggle("on", pressed);
  }
}

// Solo se consulta el estado con la ventana visible, para no gastar nada
// cuando está minimizada o tapada.
async function liveLoop() {
  if (!document.hidden && profile) {
    try {
      updateLive(await invoke("live", { profile }));
    } catch (error) {
      showNotice(`Error al leer el estado: ${error}`, "error");
    }
  }
  setTimeout(liveLoop, LIVE_INTERVAL_MS);
}

// --- Inicio -----------------------------------------------------------------

function defaultProfile() {
  const stick = {
    modo: "stick",
    zona_muerta: 0.25,
    zona_exterior: 1,
    curva: 1,
    invertir_y: false,
    velocidad_raton: 1500,
  };
  return {
    intervalo_ms: 1,
    juegos: [],
    stick_izquierdo: { ...stick },
    stick_derecho: { ...stick },
    gatillos: { umbral: 30 },
    vibracion: { intensidad: 1 },
    botones: {},
    turbo: {},
    macros: {},
  };
}

async function init() {
  options = await invoke("options");
  buildSticks();
  buildButtons();
  bindInputs();
  bindGames();
  bindMacros();
  bindProfiles();
  await bindGeneral();

  const info = await refreshProfiles(options.default_profile);
  // Se abre el perfil base, que es el que se usa la mayor parte del tiempo.
  await openProfile(info.names.includes(info.base) ? info.base : options.default_profile);

  $("#save").addEventListener("click", save);
  $("#discard").addEventListener("click", discard);
  $("#test-vibration").addEventListener("click", () => invoke("test_vibration"));
  document.addEventListener("keydown", (event) => {
    if (event.ctrlKey && event.key.toLowerCase() === "s" && isDirty()) {
      event.preventDefault();
      save();
    }
  });

  liveLoop();
}

init();
