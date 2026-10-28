// Ventana de configuración: edita el perfil y muestra el mando en vivo.
const { invoke } = window.__TAURI__.core;

const LIVE_INTERVAL_MS = 33;
const MOUSE_ACTIONS = [
  ["raton:izquierdo", "Clic izquierdo"],
  ["raton:derecho", "Clic derecho"],
  ["raton:medio", "Clic central"],
];

let options = null;
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

    row.append(dot, name, select);
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
    const key = Object.keys(profile.botones).find((k) => k.toLowerCase() === name.toLowerCase());
    selectAction($("select", row), key ? profile.botones[key] : "");
  }
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
    row.classList.toggle("changed", $("select", row).value !== "");
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
  setSaveStatus(dirty ? "Cambios sin guardar" : "Sin cambios", dirty ? "dirty" : "");
}

async function save() {
  try {
    await invoke("save_profile", { profile });
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

function updateLive(view) {
  const state = view.state;

  if (!view.service) {
    setPill("#pill-service", "bad", "Servicio detenido");
    setPill("#pill-pad", "", "Mando");
    setPill("#pill-virtual", "", "Mando virtual");
  } else {
    setPill("#pill-service", "ok", state.enabled ? "Servicio activo" : "Servicio (perfil en pausa)");
    setPill("#pill-pad", state.connected ? "ok" : "warn", state.connected ? "Mando conectado" : "Sin mando");
    const virtual = [
      ["", "Mando virtual"],
      ["ok", "Mando virtual activo"],
      ["bad", "Falta ViGEmBus"],
      ["bad", "Error del mando virtual"],
    ][state.virtual_pad] || ["", "Mando virtual"];
    setPill("#pill-virtual", virtual[0], virtual[1]);
  }

  if (view.error) {
    showNotice(`El perfil tiene un error y no se puede guardar: ${view.error}`, "error");
  } else if (!view.service) {
    showNotice("El servicio no está en marcha. Puedes editar el perfil, pero no verás el mando en vivo ni se aplicarán los cambios hasta que lo inicies.");
  } else if (!state.profile_ok) {
    showNotice("El servicio no pudo cargar el perfil guardado y sigue usando el anterior. Guarda desde aquí para corregirlo.", "error");
  } else {
    showNotice("");
  }

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
  if (!document.hidden) {
    try {
      updateLive(await invoke("live", { profile }));
    } catch (error) {
      showNotice(`Error al leer el estado: ${error}`, "error");
    }
  }
  setTimeout(liveLoop, LIVE_INTERVAL_MS);
}

// --- Inicio -----------------------------------------------------------------

async function init() {
  options = await invoke("options");
  buildSticks();
  buildButtons();
  bindInputs();

  try {
    saved = await invoke("load_profile");
  } catch (error) {
    showNotice(`No se pudo leer el perfil (${options.profile_path}): ${error}. Se muestran los valores por defecto; al guardar se reemplazará.`, "error");
    saved = null;
  }
  profile = clone(saved ?? defaultProfile());
  if (!saved) saved = clone(profile);

  fillForm();
  onChange();

  $("#save").addEventListener("click", save);
  $("#discard").addEventListener("click", discard);
  $("#test-vibration").addEventListener("click", () => invoke("test_vibration"));

  // Se aplica al momento: no forma parte del perfil.
  const autostart = $("#autostart");
  const autostartError = $("#autostart-error");
  autostart.checked = await invoke("autostart_enabled");
  autostart.addEventListener("change", async () => {
    try {
      autostart.checked = await invoke("set_autostart", { enabled: autostart.checked });
      autostartError.hidden = true;
    } catch (error) {
      autostart.checked = await invoke("autostart_enabled");
      autostartError.textContent = String(error);
      autostartError.hidden = false;
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.ctrlKey && event.key.toLowerCase() === "s" && isDirty()) {
      event.preventDefault();
      save();
    }
  });

  liveLoop();
}

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
    stick_izquierdo: { ...stick },
    stick_derecho: { ...stick },
    gatillos: { umbral: 30 },
    vibracion: { intensidad: 1 },
    botones: {},
  };
}

init();
