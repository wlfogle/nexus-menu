'use strict';

// Everything shown here comes from files on this machine (program names,
// desktop-entry names, paths), so none of it is ever put into the page as
// HTML. Elements are built with createElement and filled with textContent.

const tauri = window.__TAURI__ && window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const state = {
  scan: null, // { items, summary, target_dir }
  selected: new Set(), // ids ticked on the Missing tab
  filter: '',
  generated: null, // entries on the Created tab
  generatedSelected: new Set(),
  prefixes: null,
  busy: 0,
};

// --- helpers ---------------------------------------------------------------

function el(tag, attrs, ...children) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs || {})) {
    if (key === 'class') node.className = value;
    else if (key === 'text') node.textContent = value;
    else if (value !== false && value !== null && value !== undefined) {
      node.setAttribute(key, value === true ? '' : String(value));
    }
  }
  for (const child of children.flat()) {
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

function replace(parent, ...children) {
  parent.replaceChildren(...children.flat());
}

function plural(n, one, many) {
  return `${n} ${n === 1 ? one : many}`;
}

async function call(command, args) {
  if (!tauri) throw new Error('This page only works inside the nexus-menu-gui window.');
  return tauri.invoke(command, args);
}

function showError(error) {
  const box = $('error');
  const message = typeof error === 'string' ? error : (error && error.message) || String(error);
  replace(box, el('span', { text: message }), el('button', { type: 'button', text: 'Dismiss', id: 'dismiss' }));
  box.hidden = false;
  $('dismiss').addEventListener('click', () => { box.hidden = true; });
}

function clearError() {
  $('error').hidden = true;
}

// Runs `work` behind the spinner. The spinner stays up until every overlapping
// call has finished.
async function busy(text, work) {
  state.busy += 1;
  $('busy-text').textContent = text;
  $('busy').hidden = false;
  try {
    return await work();
  } finally {
    state.busy -= 1;
    if (state.busy === 0) $('busy').hidden = true;
  }
}

function groupLabel(group) {
  return group === 'programs' ? 'Programs on your PATH' : `Windows apps: ${group}`;
}

// --- tabs --------------------------------------------------------------------

function showTab(name) {
  for (const tab of document.querySelectorAll('.tab')) {
    tab.setAttribute('aria-selected', String(tab.dataset.tab === name));
  }
  for (const section of document.querySelectorAll('main > section')) {
    section.hidden = section.id !== `tab-${name}`;
  }
  if (name === 'created' && state.generated === null) loadGenerated();
  if (name === 'prefixes' && state.prefixes === null) loadPrefixes();
}

// --- Missing -----------------------------------------------------------------

function visibleItems() {
  if (!state.scan) return [];
  const words = state.filter.toLowerCase().split(/\s+/).filter(Boolean);
  return state.scan.items.filter((item) => {
    const hay = `${item.name} ${item.group} ${item.detail}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

function renderSummary() {
  const s = state.scan && state.scan.summary;
  if (!s) return replace($('summary'));
  const parts = [
    `${s.installed} catalogued programs installed, ${s.in_menu} already in the menu`,
    `${s.windows_apps} Windows apps in ${plural(s.windows_prefixes, 'prefix', 'prefixes')}, ${s.windows_in_menu} already in the menu`,
  ];
  if (s.windows_no_launcher > 0) parts.push(`${s.windows_no_launcher} need a launcher that is not installed`);
  if (s.windows_left_alone > 0) parts.push(`${plural(s.windows_left_alone, 'prefix', 'prefixes')} left alone`);
  $('summary').textContent = `${parts.join('; ')}. Entries go to ${state.scan.target_dir}.`;
}

function renderList() {
  const list = $('list');
  if (!state.scan) return replace(list);

  const items = visibleItems();
  if (items.length === 0) {
    const text = state.scan.items.length === 0
      ? 'Everything you have installed already has a menu entry.'
      : 'Nothing matches the filter.';
    replace(list, el('div', { class: 'empty', text }));
    return updateCount();
  }

  const groups = new Map();
  for (const item of items) {
    if (!groups.has(item.group)) groups.set(item.group, []);
    groups.get(item.group).push(item);
  }
  const order = [...groups.keys()].sort((a, b) =>
    a === 'programs' ? -1 : b === 'programs' ? 1 : a.localeCompare(b));

  const nodes = [];
  groupHeads = [];
  for (const group of order) {
    const members = groups.get(group);
    const head = el('input', { type: 'checkbox', 'aria-label': `Select all in ${groupLabel(group)}` });
    head.addEventListener('change', () => {
      for (const m of members) {
        if (head.checked) state.selected.add(m.id); else state.selected.delete(m.id);
      }
      renderList();
    });
    groupHeads.push({ head, members });
    nodes.push(el('div', { class: 'group-head' }, head,
      el('span', { text: groupLabel(group) }),
      el('span', { class: 'muted', text: `${members.length}` })));

    for (const item of members) {
      const box = el('input', { type: 'checkbox' });
      box.checked = state.selected.has(item.id);
      box.addEventListener('change', () => {
        if (box.checked) state.selected.add(item.id); else state.selected.delete(item.id);
        updateCount();
        syncGroupHeads();
      });
      nodes.push(el('div', { class: 'row' },
        el('label', {}, box,
          el('span', { class: 'name', text: item.name }),
          el('span', { class: 'detail', text: item.detail, title: item.detail }))));
    }
  }
  replace(list, nodes);
  syncGroupHeads();
  updateCount();
}

// The group checkboxes of the rows on screen, so ticking one row can update
// them without redrawing the list (which would lose focus and scroll).
let groupHeads = [];

function syncGroupHeads() {
  for (const { head, members } of groupHeads) {
    const ticked = members.filter((m) => state.selected.has(m.id)).length;
    head.checked = ticked === members.length;
    head.indeterminate = ticked > 0 && ticked < members.length;
  }
}

function updateCount() {
  const n = state.selected.size;
  $('count').textContent = n === 0 ? '' : `${n} selected`;
  $('create').disabled = n === 0;
}

function statusTag(result) {
  switch (result.status) {
    case 'created': return ['created', 'ok'];
    case 'would_create': return ['would create', 'ok'];
    case 'exists': return ['already exists, skipped (tick Overwrite to replace)', 'warn'];
    default: return [`failed: ${result.message || 'unknown error'}`, 'bad'];
  }
}

function showResults(box, rows) {
  replace(box, rows);
  box.hidden = rows.length === 0;
}

async function loadScan() {
  clearError();
  try {
    state.scan = await busy('Looking for applications without a menu entry', () => call('scan'));
  } catch (error) {
    state.scan = null;
    showError(error);
  }
  const known = new Set(state.scan ? state.scan.items.map((i) => i.id) : []);
  state.selected = new Set([...state.selected].filter((id) => known.has(id)));
  renderSummary();
  renderList();
}

async function createSelected() {
  clearError();
  const ids = [...state.selected];
  const dryRun = $('dry').checked;
  let results;
  try {
    results = await busy(dryRun ? 'Checking' : 'Creating entries',
      () => call('install', { ids, force: $('force').checked, dryRun }));
  } catch (error) {
    return showError(error);
  }

  showResults($('results'), results.map((r) => {
    const [text, kind] = statusTag(r);
    return el('div', { class: 'row' },
      el('span', { class: 'name', text: r.name }),
      el('span', { class: 'detail', text: r.path || '', title: r.path || '' }),
      el('span', { class: `tag ${kind}`, text }));
  }));

  if (!dryRun && results.some((r) => r.status === 'created')) {
    for (const r of results) if (r.status === 'created') state.selected.delete(r.id);
    state.generated = null; // the Created tab is out of date
    await loadScan();
  }
}

// --- Created -----------------------------------------------------------------

function updateRemove() {
  const n = state.generatedSelected.size;
  $('created-count').textContent = n === 0 ? '' : `${n} selected`;
  $('remove').disabled = n === 0;
  disarmRemove();
}

let disarmTimer = null;
function disarmRemove() {
  clearTimeout(disarmTimer);
  const button = $('remove');
  button.classList.remove('armed');
  button.textContent = 'Remove selected';
}

function renderGenerated() {
  const list = $('created-list');
  const entries = state.generated || [];
  $('created-summary').textContent = entries.length === 0
    ? ''
    : `${plural(entries.length, 'entry', 'entries')} created by nexus-menu. Removing deletes the .desktop file only, never the program.`;
  if (entries.length === 0) {
    replace(list, el('div', { class: 'empty', text: 'nexus-menu has not created any entries.' }));
    return updateRemove();
  }
  replace(list, entries.map((entry) => {
    const box = el('input', { type: 'checkbox' });
    box.checked = state.generatedSelected.has(entry.id);
    box.addEventListener('change', () => {
      if (box.checked) state.generatedSelected.add(entry.id); else state.generatedSelected.delete(entry.id);
      updateRemove();
    });
    return el('div', { class: 'row' },
      el('label', {}, box,
        el('span', { class: 'name', text: entry.name }),
        el('span', { class: 'detail', text: entry.path, title: entry.path })));
  }));
  updateRemove();
}

async function loadGenerated() {
  clearError();
  try {
    state.generated = await busy('Reading created entries', () => call('list_generated'));
  } catch (error) {
    state.generated = [];
    showError(error);
  }
  const known = new Set(state.generated.map((e) => e.id));
  state.generatedSelected = new Set([...state.generatedSelected].filter((id) => known.has(id)));
  renderGenerated();
}

// Removing deletes files, so the first click arms the button and the second
// (within four seconds) does it.
async function removeSelected() {
  const button = $('remove');
  if (!button.classList.contains('armed')) {
    button.classList.add('armed');
    button.textContent = `Click again to delete ${plural(state.generatedSelected.size, 'entry', 'entries')}`;
    disarmTimer = setTimeout(disarmRemove, 4000);
    return;
  }
  disarmRemove();
  clearError();
  let results;
  try {
    results = await busy('Removing entries', () => call('remove', { ids: [...state.generatedSelected] }));
  } catch (error) {
    return showError(error);
  }
  const names = new Map((state.generated || []).map((e) => [e.id, e.name]));
  showResults($('remove-results'), results.map((r) =>
    el('div', { class: 'row' },
      el('span', { class: 'name', text: names.get(r.id) || r.id }),
      el('span', { class: `tag ${r.removed ? 'ok' : 'bad'}`, text: r.removed ? 'removed' : `not removed: ${r.message || 'unknown error'}` }))));
  for (const r of results) if (r.removed) state.generatedSelected.delete(r.id);
  await loadGenerated();
  if (results.some((r) => r.removed)) await loadScan();
}

// --- Prefixes ----------------------------------------------------------------

async function loadPrefixes() {
  clearError();
  try {
    state.prefixes = await busy('Searching for Wine prefixes', () => call('prefixes'));
  } catch (error) {
    state.prefixes = [];
    showError(error);
  }
  $('prefix-summary').textContent = `${plural(state.prefixes.length, 'Wine prefix', 'Wine prefixes')} found. The owner is the tool that manages it; apps in a launcher's prefix are started by that launcher.`;
  replace($('prefix-list'), state.prefixes.length === 0
    ? el('div', { class: 'empty', text: 'No Wine prefixes found.' })
    : state.prefixes.map((p) => el('div', { class: 'row' },
      el('span', { class: 'name', text: p.owner }),
      el('span', { class: 'detail', text: p.path, title: p.path }))));
}

// --- updating ----------------------------------------------------------------

// Asks whether a newer version exists. This runs quietly in the background: if
// there is no network, or the check fails for any reason, nothing is shown.
async function checkUpdate() {
  let view;
  try {
    view = await call('check_update');
  } catch (error) {
    console.warn('update check failed:', error);
    return;
  }
  $('version').textContent = `v${view.current}`;
  if (!view.available) return;

  const banner = $('banner');
  const message = el('span', {
    text: `Version ${view.available.version} is available (you have ${view.current}).`,
    title: view.available.notes,
  });
  if (view.can_install) {
    const button = el('button', { type: 'button', text: 'Install and restart' });
    button.addEventListener('click', installUpdate);
    replace(banner, message, button);
  } else {
    replace(banner, message,
      el('span', { text: `${view.problem || 'This copy cannot update itself.'} Get it from ${view.releases_url}` }));
  }
  banner.hidden = false;
}

// On success the app starts the new version and quits, so there is nothing to
// do afterwards.
async function installUpdate() {
  clearError();
  try {
    await busy('Downloading and verifying the update', () => call('install_update'));
  } catch (error) {
    showError(error);
  }
}

// --- wiring ------------------------------------------------------------------

function selectVisible(on) {
  for (const item of visibleItems()) {
    if (on) state.selected.add(item.id); else state.selected.delete(item.id);
  }
  renderList();
}

async function rescan() {
  state.generated = null;
  state.prefixes = null;
  await loadScan();
  const open = document.querySelector('.tab[aria-selected="true"]').dataset.tab;
  if (open !== 'missing') showTab(open);
}

document.addEventListener('DOMContentLoaded', () => {
  for (const tab of document.querySelectorAll('.tab')) {
    tab.addEventListener('click', () => showTab(tab.dataset.tab));
  }
  $('rescan').addEventListener('click', rescan);
  $('filter').addEventListener('input', (event) => { state.filter = event.target.value; renderList(); });
  $('sel-all').addEventListener('click', () => selectVisible(true));
  $('sel-none').addEventListener('click', () => selectVisible(false));
  $('create').addEventListener('click', createSelected);
  $('remove').addEventListener('click', removeSelected);
  loadScan();
  checkUpdate();
});
