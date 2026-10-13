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
  entries: null, // every menu entry, on the Entries tab
  entryOrigin: 'created', // which origin the Entries tab shows
  entryFilter: '',
  entrySelected: new Set(), // created entries ticked for bulk removal
  editing: null, // id of the entry open in the editor
  scanStale: false, // something changed that the Missing tab should recheck
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
  if (name === 'missing' && state.scanStale) {
    state.scanStale = false;
    loadScan();
  }
  if (name === 'entries' && state.entries === null) loadEntries();
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
    state.entries = null; // the Entries tab is out of date
    await loadScan();
  }
}

// --- Entries -----------------------------------------------------------------

// Every application entry on this machine: the ones nexus-menu created, your
// own, and the system's. Each can be edited, and removed in the way that fits
// where it came from. The page names an entry by its id only; the engine finds
// the file.

const ORIGINS = [
  ['created', 'Created by nexus-menu'],
  ['yours', 'Yours'],
  ['system', 'System'],
  ['all', 'All'],
];

function buildOriginButtons() {
  replace($('entries-origins'), ORIGINS.map(([origin, label]) => {
    const button = el('button', {
      type: 'button',
      text: label,
      'data-origin': origin,
      'aria-pressed': String(origin === state.entryOrigin),
    });
    button.addEventListener('click', () => {
      state.entryOrigin = origin;
      renderEntries();
    });
    return button;
  }));
}

function visibleEntries() {
  const words = state.entryFilter.toLowerCase().split(/\s+/).filter(Boolean);
  return (state.entries || []).filter((entry) => {
    if (state.entryOrigin !== 'all' && entry.origin !== state.entryOrigin) return false;
    const hay = `${entry.name} ${entry.id}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

function removeLabel(entry) {
  return entry.origin === 'system' ? 'Hide' : 'Remove';
}

function removeHint(entry) {
  if (entry.origin === 'created') {
    return 'Deletes the .desktop file nexus-menu made. The program itself is not touched.';
  }
  if (entry.origin === 'yours') {
    return 'Moves your .desktop file to the backup folder. Restore original brings it back.';
  }
  return 'Hides it from the menu with a copy in your own folder. The system file is not changed, and Restore original undoes it.';
}

function removalText(removal) {
  return ({
    deleted: 'deleted',
    moved_to_backup: 'moved to the backup folder (Restore original brings it back)',
    hidden: 'hidden from the menu (Restore original undoes it)',
    already_hidden: 'was already hidden',
  })[removal] || removal;
}

function restoredText(restored) {
  return ({
    override_removed: 'restored: your edited copy was deleted and the system entry is back',
    original_restored: 'restored: your file is back as it was before the first edit',
    removed_restored: 'restored: your removed file is back',
  })[restored] || restored;
}

function badges(entry) {
  const out = [];
  if (entry.removed) out.push(['Removed', 'warn']);
  if (entry.hidden) out.push(['Hidden', 'warn']);
  if (entry.edited) out.push(['Edited', 'ok']);
  if (entry.stale) out.push(['System entry changed since your edit', 'bad']);
  return out.map(([text, kind]) => el('span', { class: `badge ${kind}`, text }));
}

// Anything that deletes or hides takes two clicks: the first arms the button,
// the second (within four seconds) does it.
let armed = null;

function disarm() {
  if (!armed) return;
  clearTimeout(armed.timer);
  armed.button.classList.remove('armed');
  armed.button.textContent = armed.label;
  armed = null;
}

function twoClick(key, button, then) {
  button.addEventListener('click', () => {
    if (armed && armed.key === key) {
      disarm();
      then();
      return;
    }
    disarm();
    armed = { key, button, label: button.textContent, timer: setTimeout(disarm, 4000) };
    button.classList.add('armed');
    button.textContent = 'Click again to confirm';
  });
}

function entryRow(entry) {
  const main = el('div', { class: 'entry-main' });
  if (entry.origin === 'created' && !entry.removed) {
    const box = el('input', { type: 'checkbox', 'aria-label': `Select ${entry.name}` });
    box.checked = state.entrySelected.has(entry.id);
    box.addEventListener('change', () => {
      if (box.checked) state.entrySelected.add(entry.id); else state.entrySelected.delete(entry.id);
      updateEntrySelection();
    });
    main.append(box);
  }
  main.append(
    el('span', { class: 'name', text: entry.name }),
    ...badges(entry),
    el('span', { class: 'detail', text: entry.id, title: entry.id }),
  );

  const actions = el('span', { class: 'row-actions' });
  if (!entry.removed) {
    const edit = el('button', { type: 'button', class: 'small', text: 'Edit', title: 'Change the name, command, icon and more' });
    edit.addEventListener('click', () => openEditor(entry));
    actions.append(edit);
    if (!entry.hidden) {
      const remove = el('button', { type: 'button', class: 'small danger', text: removeLabel(entry), title: removeHint(entry) });
      twoClick(entry.id, remove, () => removeEntries([entry.id]));
      actions.append(remove);
    }
  }
  if (entry.restorable) {
    const restore = el('button', { type: 'button', class: 'small', text: 'Restore original', title: 'Undo what was done to this entry' });
    restore.addEventListener('click', () => restoreEntry(entry.id));
    actions.append(restore);
  }
  return el('div', { class: entry.removed ? 'row removed' : 'row' }, main, actions);
}

function renderEntries() {
  const list = $('entries-list');
  const all = state.entries || [];
  const count = (origin) => all.filter((e) => e.origin === origin).length;
  $('entries-summary').textContent = all.length === 0
    ? ''
    : `${plural(all.length, 'entry', 'entries')}: ${count('created')} created by nexus-menu, ${count('yours')} yours, ${count('system')} from the system. Editing a system entry writes a copy in your own folder; the system file is never changed.`;

  for (const button of document.querySelectorAll('#entries-origins button')) {
    button.setAttribute('aria-pressed', String(button.dataset.origin === state.entryOrigin));
  }

  const shown = visibleEntries();
  if (shown.length === 0) {
    const text = all.length === 0
      ? 'No entries were found.'
      : state.entryOrigin === 'created' && state.entryFilter === ''
        ? 'nexus-menu has not created any entries. Pick Yours, System or All to see the others.'
        : 'Nothing matches.';
    replace(list, el('div', { class: 'empty', text }));
    return updateEntrySelection();
  }
  disarm();
  replace(list, shown.map(entryRow));
  updateEntrySelection();
}

function updateEntrySelection() {
  const n = state.entrySelected.size;
  $('entries-count').textContent = n === 0 ? '' : `${n} selected`;
  $('entries-remove-selected').disabled = n === 0;
}

async function loadEntries() {
  clearError();
  try {
    state.entries = await busy('Reading menu entries', () => call('list_entries'));
  } catch (error) {
    state.entries = [];
    showError(error);
  }
  const known = new Set(state.entries.filter((e) => e.origin === 'created').map((e) => e.id));
  state.entrySelected = new Set([...state.entrySelected].filter((id) => known.has(id)));
  renderEntries();
}

async function removeEntries(ids) {
  clearError();
  let results;
  try {
    results = await busy('Removing', () => call('remove_entries', { ids }));
  } catch (error) {
    return showError(error);
  }
  const names = new Map((state.entries || []).map((e) => [e.id, e.name]));
  showResults($('entries-results'), results.map((r) =>
    el('div', { class: 'row' },
      el('span', { class: 'name', text: names.get(r.id) || r.id }),
      el('span', { class: `tag ${r.removal ? 'ok' : 'bad'}`, text: r.removal ? removalText(r.removal) : `not removed: ${r.message || 'unknown error'}` }))));
  for (const r of results) if (r.removal) state.entrySelected.delete(r.id);
  if (results.some((r) => r.removal)) state.scanStale = true;
  await loadEntries();
}

async function restoreEntry(id) {
  clearError();
  let restored;
  try {
    restored = await busy('Restoring', () => call('restore_entry', { id }));
  } catch (error) {
    return showError(error);
  }
  const names = new Map((state.entries || []).map((e) => [e.id, e.name]));
  showResults($('entries-results'), [el('div', { class: 'row' },
    el('span', { class: 'name', text: names.get(id) || id }),
    el('span', { class: 'tag ok', text: restoredText(restored) }))]);
  state.scanStale = true;
  await loadEntries();
}

// --- the editor ---

const FIELD_INPUTS = {
  name: 'ef-name',
  comment: 'ef-comment',
  exec: 'ef-exec',
  icon: 'ef-icon',
  categories: 'ef-categories',
  keywords: 'ef-keywords',
};

function clearFieldErrors() {
  for (const node of document.querySelectorAll('.field-error')) node.textContent = '';
}

// Once a field is touched, what was said about it no longer applies.
function clearFieldError(field) {
  const node = document.querySelector(`.field-error[data-for="${field}"]`);
  if (node) node.textContent = '';
}

async function openEditor(entry) {
  clearError();
  let detail;
  try {
    detail = await busy('Opening the entry', () => call('get_entry', { id: entry.id }));
  } catch (error) {
    return showError(error);
  }
  state.editing = detail.info.id;
  $('edit-title').textContent = `Edit ${detail.info.name}`;
  for (const [key, id] of Object.entries(FIELD_INPUTS)) $(id).value = detail.fields[key];
  $('ef-terminal').checked = detail.fields.terminal;
  $('edit-writes').textContent = `Saving writes ${detail.writes_to}`;
  replace($('edit-notes'), detail.notes.map((note) => el('li', { text: note })));
  clearFieldErrors();
  $('entries-list-view').hidden = true;
  $('entry-edit').hidden = false;
  $('ef-name').focus();
}

function closeEditor() {
  state.editing = null;
  $('entry-edit').hidden = true;
  $('entries-list-view').hidden = false;
}

function readForm() {
  const fields = {};
  for (const [key, id] of Object.entries(FIELD_INPUTS)) fields[key] = $(id).value;
  fields.terminal = $('ef-terminal').checked;
  return fields;
}

async function saveEditor() {
  clearError();
  clearFieldErrors();
  let saved;
  try {
    saved = await busy('Saving', () => call('save_entry', { id: state.editing, fields: readForm() }));
  } catch (error) {
    if (error && error.kind === 'invalid') {
      for (const problem of error.detail) {
        const node = document.querySelector(`.field-error[data-for="${problem.field}"]`);
        if (node) node.textContent = problem.message;
      }
      return;
    }
    return showError(error && error.kind === 'failed' ? error.detail : error);
  }
  closeEditor();
  const outcome = saved.created_override
    ? 'a copy was written to your folder; the system file is unchanged'
    : saved.changed ? 'file updated' : 'no changes';
  showResults($('entries-results'), [el('div', { class: 'row' },
    el('span', { class: 'name', text: saved.changed ? 'Saved' : 'Nothing to save' }),
    el('span', { class: 'detail', text: saved.path, title: saved.path }),
    el('span', { class: 'tag ok', text: outcome }))]);
  if (saved.changed) state.scanStale = true;
  await loadEntries();
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
  state.entries = null;
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
  buildOriginButtons();
  $('entries-filter').addEventListener('input', (event) => {
    state.entryFilter = event.target.value;
    renderEntries();
  });
  twoClick('bulk', $('entries-remove-selected'), () => removeEntries([...state.entrySelected]));
  $('edit-form').addEventListener('submit', (event) => {
    event.preventDefault();
    saveEditor();
  });
  $('edit-cancel').addEventListener('click', closeEditor);
  for (const [field, id] of Object.entries(FIELD_INPUTS)) {
    $(id).addEventListener('input', () => clearFieldError(field));
  }
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && state.editing) closeEditor();
  });
  loadScan();
  checkUpdate();
});
