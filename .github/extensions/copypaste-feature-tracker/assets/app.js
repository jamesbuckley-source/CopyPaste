const STATUS_HINTS = {
  planned: "Agreed, not started.",
  "in-progress": "Being built now.",
  blocked: "Waiting on a dependency.",
  shipped: "Implemented and released.",
};

const dom = {
  artifact: document.getElementById("artifact"),
  progress: document.getElementById("progress"),
  progressFill: document.getElementById("progress-fill"),
  progressLabel: document.getElementById("progress-label"),
  counts: document.getElementById("counts"),
  search: document.getElementById("search"),
  statusFilter: document.getElementById("status-filter"),
  areaFilter: document.getElementById("area"),
  showEmpty: document.getElementById("show-empty"),
  boardMeta: document.getElementById("board-meta"),
  status: document.getElementById("status"),
  board: document.getElementById("board"),
  dialog: document.getElementById("editor"),
  form: document.getElementById("editor-form"),
  editorTitle: document.getElementById("editor-title"),
  fieldTitle: document.getElementById("field-title"),
  fieldArea: document.getElementById("field-area"),
  fieldStatus: document.getElementById("field-status"),
  fieldSummary: document.getElementById("field-summary"),
  fieldOwner: document.getElementById("field-owner"),
  fieldReferences: document.getElementById("field-references"),
  formError: document.getElementById("form-error"),
  save: document.getElementById("save"),
  cancel: document.getElementById("cancel"),
  add: document.getElementById("add"),
  refresh: document.getElementById("refresh"),
  areaOptions: document.getElementById("area-options"),
};

const state = {
  data: null,
  error: null,
  search: "",
  status: "",
  area: "",
  showEmpty: true,
  appliedView: false,
  areaSignature: "",
  mode: "create",
  editingId: null,
};

const relativeTime = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function timeAgo(iso) {
  if (!iso) return "never updated";
  const timestamp = Date.parse(iso);
  if (Number.isNaN(timestamp)) return "unknown update time";
  const seconds = Math.round((timestamp - Date.now()) / 1000);
  const magnitude = Math.abs(seconds);
  if (magnitude < 60) return relativeTime.format(seconds, "second");
  if (magnitude < 3600) return relativeTime.format(Math.round(seconds / 60), "minute");
  if (magnitude < 86400) return relativeTime.format(Math.round(seconds / 3600), "hour");
  return relativeTime.format(Math.round(seconds / 86400), "day");
}

function labelFor(payload, status) {
  return payload?.statusLabels?.[status] ?? status;
}

function announce(message, isError = false) {
  dom.status.textContent = message;
  dom.status.dataset.tone = isError ? "error" : "info";
}

async function load({ quiet = false } = {}) {
  let payload = null;
  try {
    const response = await fetch("/api/state", { cache: "no-store" });
    payload = await response.json().catch(() => null);
    if (!response.ok || !payload || payload.error) {
      state.error = payload?.error ?? {
        code: `http-${response.status}`,
        message: `The feature tracker returned status ${response.status}.`,
      };
    } else {
      state.data = payload;
      state.error = null;
      if (!state.appliedView) {
        if (payload.view?.query) state.search = payload.view.query;
        if (payload.view?.status) state.status = payload.view.status;
        state.appliedView = true;
      }
    }
  } catch (error) {
    state.error = {
      code: "network",
      message: `Could not reach the feature tracker: ${error instanceof Error ? error.message : String(error)}`,
    };
  }
  render({ quiet });
}

function filteredFeatures() {
  const features = state.data?.features ?? [];
  const needle = state.search.trim().toLowerCase();
  return features.filter((feature) => {
    if (state.status && feature.status !== state.status) return false;
    if (state.area && feature.area !== state.area) return false;
    if (!needle) return true;
    const haystack = [feature.id, feature.title, feature.area, feature.summary, feature.owner, ...(feature.references ?? [])]
      .join("\n")
      .toLowerCase();
    return haystack.includes(needle);
  });
}

function render({ quiet = false } = {}) {
  const payload = state.data;
  if (state.error && !payload) {
    dom.artifact.textContent = state.error.artifactPath ?? "docs/features.json";
    dom.progressFill.style.width = "0%";
    dom.progress.setAttribute("aria-valuenow", "0");
    dom.progressLabel.textContent = "Board unavailable";
    dom.counts.replaceChildren();
    dom.boardMeta.textContent = "";
    dom.board.replaceChildren(errorPanel(state.error));
    if (!quiet) announce(state.error.message, true);
    return;
  }
  if (!payload) return;

  dom.artifact.textContent = payload.artifactPath;
  dom.add.disabled = false;
  renderStatusOptions(payload);
  dom.statusFilter.value = state.status;
  dom.search.value = state.search;
  renderProgress(payload);
  renderAreaOptions(payload.summary.areas ?? []);

  const features = filteredFeatures();
  renderBoard(payload, features);

  const total = payload.summary.total;
  const updated = payload.updatedAt ? `Updated ${timeAgo(payload.updatedAt)}` : "Not saved yet";
  const filtering = Boolean(state.search.trim() || state.status || state.area);
  dom.boardMeta.textContent = filtering
    ? `Showing ${features.length} of ${total} features · ${updated}`
    : `${total} ${total === 1 ? "feature" : "features"} · ${updated}`;
  if (state.error && !quiet) announce(state.error.message, true);
}

function errorPanel(error) {
  const panel = el("div", "error-panel");
  panel.append(el("h2", null, "The board could not be loaded"));
  panel.append(el("p", null, error.message));
  if (error.artifactPath) {
    const path = el("p", "hint");
    path.append(el("code", null, error.artifactPath));
    panel.append(path);
  }
  const hint = el(
    "p",
    "hint",
    "Fix the file by hand, or delete it to start a new board. The canvas never overwrites a file it cannot read.",
  );
  panel.append(hint);
  return panel;
}

function renderStatusOptions(payload) {
  if (dom.statusFilter.options.length <= 1) {
    for (const status of payload.statuses) {
      const option = el("option", null, labelFor(payload, status));
      option.value = status;
      dom.statusFilter.append(option);
    }
  }
  if (dom.fieldStatus.options.length === 0) {
    for (const status of payload.statuses) {
      const option = el("option", null, labelFor(payload, status));
      option.value = status;
      dom.fieldStatus.append(option);
    }
  }
}

function renderProgress(payload) {
  const { summary } = payload;
  dom.progressFill.style.width = `${summary.shippedPercent}%`;
  dom.progress.setAttribute("aria-valuenow", String(summary.shippedPercent));
  dom.progressLabel.textContent =
    summary.total === 0
      ? "No features yet — add the first one."
      : `${summary.counts.shipped} of ${summary.total} shipped (${summary.shippedPercent}%)`;

  dom.counts.replaceChildren(
    ...payload.statuses.map((status) => {
      const item = el("li", `count count-${status}`);
      const value = summary.counts[status] ?? 0;
      item.append(el("span", "count-value", String(value)));
      item.append(el("span", "count-label", labelFor(payload, status)));
      item.append(el("span", "count-hint", STATUS_HINTS[status] ?? ""));
      return item;
    }),
  );
}

function renderAreaOptions(areas) {
  const signature = areas.map((entry) => entry.area).join("\u0000");
  if (signature === state.areaSignature) return;
  state.areaSignature = signature;

  if (state.area && !areas.some((entry) => entry.area === state.area)) {
    state.area = "";
  }

  dom.areaFilter.replaceChildren(el("option", null, "All areas"));
  dom.areaFilter.firstElementChild.value = "";
  dom.areaOptions.replaceChildren();
  for (const entry of areas) {
    const option = el("option", null, entry.area);
    option.value = entry.area;
    dom.areaFilter.append(option);
    const suggestion = el("option");
    suggestion.value = entry.area;
    dom.areaOptions.append(suggestion);
  }
  dom.areaFilter.value = state.area;
}

function renderBoard(payload, features) {
  const columns = [];
  for (const status of payload.statuses) {
    if (state.status && state.status !== status) continue;
    const columnFeatures = features.filter((feature) => feature.status === status);
    if (columnFeatures.length === 0 && !state.showEmpty) continue;

    const column = el("section", "column");
    column.dataset.status = status;
    column.setAttribute("aria-labelledby", `column-${status}`);

    const heading = el("h2", "column-title");
    heading.id = `column-${status}`;
    heading.append(el("span", null, labelFor(payload, status)));
    heading.append(el("span", "column-count", String(columnFeatures.length)));
    column.append(heading);

    if (columnFeatures.length === 0) {
      column.append(el("p", "empty", state.search.trim() || state.area ? "Nothing matches the filters." : "Nothing here yet."));
    } else {
      const list = el("div", "cards");
      for (const feature of columnFeatures) list.append(renderCard(feature, payload));
      column.append(list);
    }
    columns.push(column);
  }

  if (columns.length === 0) {
    const empty = el("div", "board-empty");
    empty.append(el("h2", null, payload.summary.total === 0 ? "No features tracked yet" : "No matching features"));
    empty.append(
      el(
        "p",
        null,
        payload.summary.total === 0
          ? "Add a feature here, or ask Copilot to add one with the add_feature canvas action."
          : "Clear the search or area filter to see the rest of the board.",
      ),
    );
    columns.push(empty);
  }

  dom.board.replaceChildren(...columns);
}

function renderCard(feature, payload) {
  const card = el("article", "card");
  card.dataset.status = feature.status;

  const head = el("div", "card-head");
  head.append(el("h3", "card-title", feature.title));

  const statusSelect = document.createElement("select");
  statusSelect.className = "status-select";
  statusSelect.setAttribute("aria-label", `Status for ${feature.title}`);
  for (const status of payload.statuses) {
    const option = el("option", null, labelFor(payload, status));
    option.value = status;
    statusSelect.append(option);
  }
  statusSelect.value = feature.status;
  statusSelect.addEventListener("change", () => changeStatus(feature, statusSelect));
  head.append(statusSelect);
  card.append(head);

  const meta = el("p", "card-meta");
  const area = el("span", "chip", feature.area || "General");
  meta.append(area);
  const updated = el("span", "updated", `Updated ${timeAgo(feature.updatedAt)}`);
  if (feature.updatedAt) updated.title = feature.updatedAt;
  meta.append(updated);
  if (feature.owner) meta.append(el("span", "owner", feature.owner));
  card.append(meta);

  if (feature.summary) card.append(el("p", "card-summary", feature.summary));

  const references = feature.references ?? [];
  if (references.length > 0) {
    const list = el("ul", "refs");
    for (const reference of references) {
      const item = el("li");
      item.append(el("code", null, reference));
      list.append(item);
    }
    card.append(list);
  }

  const footer = el("div", "card-actions");
  const edit = el("button", "btn tiny", "Edit");
  edit.type = "button";
  edit.setAttribute("aria-label", `Edit ${feature.title}`);
  edit.addEventListener("click", () => openEditor(feature));
  footer.append(edit);

  const remove = el("button", "btn tiny danger", "Delete");
  remove.type = "button";
  remove.setAttribute("aria-label", `Delete ${feature.title}`);
  let confirmTimer = 0;
  remove.addEventListener("click", () => {
    if (remove.dataset.confirming === "true") {
      clearTimeout(confirmTimer);
      removeFeature(feature);
      return;
    }
    remove.dataset.confirming = "true";
    remove.textContent = "Confirm delete";
    confirmTimer = setTimeout(() => {
      if (!remove.isConnected) return;
      remove.dataset.confirming = "false";
      remove.textContent = "Delete";
    }, 4000);
  });
  footer.append(remove);

  card.append(footer);
  return card;
}

async function changeStatus(feature, select) {
  select.disabled = true;
  try {
    const body = await request(`/api/features/${encodeURIComponent(feature.id)}`, {
      method: "PATCH",
      body: { status: select.value },
    });
    state.data = body;
    render();
    announce(`“${feature.title}” moved to ${labelFor(state.data, select.value)}.`);
  } catch (error) {
    select.disabled = false;
    select.value = feature.status;
    announce(error.message, true);
  }
}

async function removeFeature(feature) {
  try {
    const body = await request(`/api/features/${encodeURIComponent(feature.id)}`, { method: "DELETE" });
    state.data = body;
    render();
    announce(`“${feature.title}” removed.`);
  } catch (error) {
    announce(error.message, true);
  }
}

async function request(url, { method = "GET", body } = {}) {
  const response = await fetch(url, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    cache: "no-store",
  });
  const payload = await response.json().catch(() => null);
  if (!response.ok) {
    throw new Error(payload?.error?.message ?? `Request failed with status ${response.status}.`);
  }
  return payload;
}

function openEditor(feature) {
  state.mode = feature ? "edit" : "create";
  state.editingId = feature?.id ?? null;
  dom.editorTitle.textContent = feature ? `Edit “${feature.title}”` : "Add feature";
  dom.fieldTitle.value = feature?.title ?? "";
  dom.fieldArea.value = feature?.area ?? (state.area || "");
  dom.fieldStatus.value = feature?.status ?? "planned";
  dom.fieldSummary.value = feature?.summary ?? "";
  dom.fieldOwner.value = feature?.owner ?? "";
  dom.fieldReferences.value = (feature?.references ?? []).join("\n");
  dom.formError.textContent = "";
  dom.dialog.showModal();
  dom.fieldTitle.focus();
  dom.fieldTitle.select();
}

async function save(event) {
  event.preventDefault();
  const payload = {
    title: dom.fieldTitle.value.trim(),
    area: dom.fieldArea.value.trim() || "General",
    status: dom.fieldStatus.value,
    summary: dom.fieldSummary.value.trim(),
    owner: dom.fieldOwner.value.trim(),
    references: dom.fieldReferences.value
      .split(/\r?\n/)
      .map((value) => value.trim())
      .filter(Boolean),
  };
  if (!payload.title) {
    dom.formError.textContent = "Give the feature a title.";
    dom.fieldTitle.focus();
    return;
  }

  const editing = state.mode === "edit";
  dom.save.disabled = true;
  try {
    const body = await request(
      editing ? `/api/features/${encodeURIComponent(state.editingId)}` : "/api/features",
      { method: editing ? "PATCH" : "POST", body: payload },
    );
    state.data = body;
    dom.dialog.close();
    render();
    announce(editing ? "Feature updated." : "Feature added.");
  } catch (error) {
    dom.formError.textContent = error.message;
  } finally {
    dom.save.disabled = false;
  }
}

function init() {
  dom.search.addEventListener("input", () => {
    state.search = dom.search.value;
    render();
  });
  dom.statusFilter.addEventListener("change", () => {
    state.status = dom.statusFilter.value;
    render();
  });
  dom.areaFilter.addEventListener("change", () => {
    state.area = dom.areaFilter.value;
    render();
  });
  dom.showEmpty.addEventListener("change", () => {
    state.showEmpty = dom.showEmpty.checked;
    render();
  });
  dom.add.addEventListener("click", () => openEditor(null));
  dom.refresh.addEventListener("click", () => load());
  dom.form.addEventListener("submit", save);
  dom.cancel.addEventListener("click", () => dom.dialog.close());
  dom.dialog.addEventListener("close", () => {
    dom.formError.textContent = "";
  });
  dom.dialog.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      dom.dialog.close();
    }
  });

  load();
  connectEvents();
}

function connectEvents() {
  const source = new EventSource("/api/events");
  source.addEventListener("state", () => {
    load({ quiet: true });
  });
}

init();
