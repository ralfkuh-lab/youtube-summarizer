import { invoke } from "@tauri-apps/api/core";
import { makeToggle } from "./dom-utils";
import { errorMessage, escapeHtml, hideModal, showModal } from "./dom-utils";

export type CatalogModel = {
    id: string;
    name?: string;
    reasoning?: boolean;
    tool_call?: boolean;
    limit?: { context?: number };
    cost?: { input?: number; output?: number };
};

export type CatalogProvider = {
    id: string;
    name?: string;
    api?: string;
    doc?: string;
    models: Record<string, CatalogModel>;
};

export type CatalogResult = {
    catalog: Record<string, CatalogProvider>;
    source: 'snapshot' | 'cache';
    updatedAt: string;
};

type ConfiguredModel = { name?: string };

export type ProviderConfig = {
    enabled: boolean;
    name?: string;
    custom?: boolean;
    options?: { baseURL: string };
    models?: Record<string, ConfiguredModel>;
    whitelist: string[];
};

export type AiConfig = {
    provider: Record<string, ProviderConfig>;
    defaultModel?: { provider: string; model: string } | null;
};

type InvokeResult<T> = { value: T; error?: never } | { value?: never; error: string };

let catalogResult: CatalogResult | null = null;
let aiConfig: AiConfig | null = null;
let authStatus: Record<string, boolean> = {};
let loadPromise: Promise<void> | null = null;

let statusModelEl: HTMLElement | null = null;
let setStatusFn: (message: string) => void = () => {};

export type AiConfigInit = {
  statusModelEl: HTMLElement;
  setStatus: (message: string) => void;
};

export function initAiConfig(deps: AiConfigInit) {
  statusModelEl = deps.statusModelEl;
  setStatusFn = deps.setStatus;
}

export function getAiConfig(): AiConfig | null {
  return aiConfig;
}

export function bindAiConfigEvents() {
  const settingsBtn = document.getElementById("settingsBtn");
  if (settingsBtn) settingsBtn.addEventListener("click", () => void openSettings());
  const closeBtn = document.getElementById("configClose");
  if (closeBtn) closeBtn.addEventListener("click", () => hideModal("#settingsModal"));

  // Tab navigation for visibility (activate); load listeners attached inside initSettingsAi
  const tabs = [
    document.getElementById("settings-tab-ki-anbieter") as HTMLButtonElement,
    document.getElementById("settings-tab-ki-modelle") as HTMLButtonElement,
  ].filter(Boolean);
  tabs.forEach((tab, index) => {
    tab.addEventListener("click", () => {
      activateSettingsTab(tab.id.replace("settings-tab-", ""));
    });
    tab.addEventListener("keydown", (e) => {
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      e.preventDefault();
      const offset = e.key === "ArrowDown" ? 1 : -1;
      const next = tabs[(index + offset + tabs.length) % tabs.length];
      next.click();
      next.focus();
    });
  });

  // Custom dialog static listeners (markup now provides the elements)
  const customCancel = document.getElementById('ai-custom-cancel');
  if (customCancel) customCancel.addEventListener('click', closeCustomDialog);
  const customForm = document.getElementById('ai-custom-form') as HTMLFormElement | null;
  if (customForm) customForm.addEventListener('submit', (ev) => { ev.preventDefault(); void saveCustomProvider(ev as Event); });
  const customDialogEl = document.getElementById('ai-custom-dialog');
  if (customDialogEl) {
    customDialogEl.addEventListener('keydown', (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape') return;
      ev.preventDefault();
      ev.stopPropagation();
      closeCustomDialog();
    });
  }

  initSettingsAi();
}

function el(id: string): HTMLElement | null {
    return document.getElementById(id);
}

function input(id: string): HTMLInputElement | null {
    return el(id) as HTMLInputElement | null;
}

function select(id: string): HTMLSelectElement | null {
    return el(id) as HTMLSelectElement | null;
}

async function invokeUi<T>(cmd: string, args: any, operation: string): Promise<InvokeResult<T>> {
    try {
        const value = await invoke<T>(cmd, args);
        return { value };
    } catch (error) {
        console.warn('settings-ai', operation, { cmd, error: String(error) });
        return { error: String(error) };
    }
}

function setError(id: string, message: string | null): void {
    const element = el(id);
    if (!element) return;
    element.textContent = message || '';
    element.hidden = !message;
}

function providerName(id: string, provider?: CatalogProvider, configured?: ProviderConfig): string {
    return configured?.name || provider?.name || id;
}

function button(text: string, className = 'settings-ai-button'): HTMLButtonElement {
    const element = document.createElement('button');
    element.type = 'button';
    element.className = className;
    element.textContent = text;
    return element;
}

function renderAuthRow(providerId: string, isCustom = false): HTMLElement {
    const row = document.createElement('div');
    row.className = 'settings-ai-auth';
    row.dataset.aiAuthProvider = providerId;

    const stored = authStatus[providerId] === true;
    const status = document.createElement('span');
    status.className = 'settings-ai-auth__status';
    if (stored || !isCustom) {
        // Bei Custom-Providern ist der Schlüssel optional — kein "fehlt"-Status.
        const dot = document.createElement('span');
        dot.className = 'settings-ai-status-dot' +
            (stored ? ' settings-ai-status-dot--stored' : '');
        dot.setAttribute('aria-hidden', 'true');
        const statusText = document.createElement('span');
        statusText.textContent = stored ? 'Schlüssel hinterlegt' : 'Schlüssel fehlt';
        status.append(dot, statusText);
    }

    const edit = button(
        stored ? 'Schlüssel ändern' : isCustom ? 'Schlüssel setzen (optional)' : 'Schlüssel setzen',
    );
    edit.id = `ai-auth-edit-${providerId}`;
    edit.dataset.aiAuthEdit = providerId;
    const remove = button('Entfernen');
    remove.id = `ai-auth-remove-${providerId}`;
    remove.dataset.aiAuthRemove = providerId;
    remove.hidden = !stored;

    const editor = document.createElement('div');
    editor.className = 'settings-ai-auth__editor';
    editor.hidden = true;
    const keyInput = document.createElement('input');
    keyInput.type = 'password';
    keyInput.id = `ai-auth-key-${providerId}`;
    keyInput.dataset.aiAuthInput = providerId;
    keyInput.className = 'settings-input';
    keyInput.autocomplete = 'new-password';
    keyInput.setAttribute('aria-label', `Schlüssel für ${providerId}`);
    const save = button('Speichern');
    save.id = `ai-auth-save-${providerId}`;
    save.dataset.aiAuthSave = providerId;
    const cancel = button('Abbrechen');
    editor.append(keyInput, save, cancel);

    const error = document.createElement('p');
    error.className = 'settings-ai-error';
    error.id = `ai-auth-error-${providerId}`;
    error.hidden = true;

    edit.addEventListener('click', () => {
        keyInput.value = '';
        editor.hidden = false;
        keyInput.focus();
    });
    cancel.addEventListener('click', () => {
        keyInput.value = '';
        editor.hidden = true;
        setError(error.id, null);
    });
    save.addEventListener('click', () => saveAuth(providerId, keyInput));
    remove.addEventListener('click', () => removeAuth(providerId, error.id));
    if (status.childElementCount > 0) row.append(status);
    row.append(edit, remove, editor, error);
    return row;
}

async function reloadAuthStatus(): Promise<boolean> {
    const result = await invokeUi<Record<string, boolean>>(
        'ai_auth_status',
        undefined,
        'KI-Schlüsselstatus konnte nicht geladen werden',
    );
    if (result.error) return false;
    authStatus = result.value || {};
    return true;
}

async function saveAuth(providerId: string, keyInput: HTMLInputElement): Promise<void> {
    const key = keyInput.value;
    const errorId = `ai-auth-error-${providerId}`;
    if (!key.trim()) {
        setError(errorId, 'Schlüssel darf nicht leer sein.');
        return;
    }
    setError(errorId, null);
    const result = await invokeUi<Record<string, boolean>>(
        'ai_auth_set',
        { providerId, key },
        'KI-Schlüssel konnte nicht gespeichert werden',
    );
    keyInput.value = '';
    if (result.value) authStatus = result.value;
    await reloadAuthStatus();
    if (result.error) {
        setError(errorId, result.error);
        return;
    }
    renderProviders();
}

async function removeAuth(providerId: string, errorId: string): Promise<void> {
    setError(errorId, null);
    const result = await invokeUi<Record<string, boolean>>(
        'ai_auth_remove',
        { providerId },
        'KI-Schlüssel konnte nicht entfernt werden',
    );
    if (result.value) authStatus = result.value;
    await reloadAuthStatus();
    if (result.error) {
        setError(errorId, result.error);
        return;
    }
    renderProviders();
}

async function setProviderEnabled(providerId: string, enabled: boolean): Promise<void> {
    const result = await invokeUi<AiConfig>(
        'ai_provider_enable',
        { providerId, enabled },
        'KI-Anbieter konnte nicht geändert werden',
    );
    if (result.error) {
        setError('ai-providers-error', result.error);
        renderProviders();
        return;
    }
    aiConfig = result.value!;
    setError('ai-providers-error', null);
    renderProviders();
    renderModels();
}

function providerCard(
    providerId: string,
    name: string,
    enabled: boolean,
    api?: string,
    doc?: string,
    isCustom = false,
): HTMLElement {
    const card = document.createElement('article');
    card.className = 'settings-ai-card';
    card.dataset.aiProviderId = providerId;

    const header = document.createElement('div');
    header.className = 'settings-ai-card__header';
    const title = document.createElement('div');
    const strong = document.createElement('strong');
    strong.textContent = name;
    const idText = document.createElement('span');
    idText.className = 'settings-ai-card__id';
    idText.textContent = providerId;
    title.append(strong, idText);
    header.append(
        title,
        makeToggle(
            `ai-provider-enabled-${providerId}`,
            enabled,
            'Aktiv',
            (checked) => setProviderEnabled(providerId, checked),
        ),
    );
    card.appendChild(header);

    if (api || doc) {
        const details = document.createElement('div');
        details.className = 'settings-ai-card__details';
        if (api) {
            const endpoint = document.createElement('span');
            endpoint.textContent = `API: ${api}`;
            details.appendChild(endpoint);
        }
        if (doc) {
            const documentation = document.createElement('span');
            documentation.textContent = `Doku: ${doc}`;
            details.appendChild(documentation);
        }
        card.appendChild(details);
    }
    card.appendChild(renderAuthRow(providerId, isCustom));
    return card;
}

// Rang für die Anbieter-Sortierung: aktive zuerst (0), dann verwendbare —
// hinterlegter Schlüssel oder Custom-Eintrag (1) —, dann der Rest (2).
// Innerhalb jeder Gruppe alphabetisch.
function providerRank(id: string): number {
    const cfg = aiConfig?.provider[id];
    if (cfg?.enabled) return 0;
    if (cfg?.custom || authStatus[id] === true) return 1;
    return 2;
}

function providerMatchesTerm(id: string, name: string, term: string): boolean {
    return !term || `${name} ${id}`.toLocaleLowerCase('de').includes(term);
}

type ProviderListEntry = {
    id: string;
    name: string;
    custom: boolean;
    api?: string;
    doc?: string;
};

function renderProviders(): void {
    const list = el('ai-provider-list');
    if (!list || !catalogResult || !aiConfig) return;
    list.textContent = '';
    const term = (input('ai-provider-search')?.value || '')
        .trim()
        .toLocaleLowerCase('de');

    // Katalog- und Custom-Provider in EINER Liste: aktiv (0) → verwendbar (1) → Rest (2).
    const entries: ProviderListEntry[] = Object.entries(catalogResult.catalog)
        .filter(([id]) => !aiConfig!.provider[id]?.custom)
        .map(([id, provider]) => ({
            id,
            name: providerName(id, provider),
            custom: false,
            api: provider.api,
            doc: provider.doc,
        }));
    for (const [id, provider] of Object.entries(aiConfig.provider)) {
        if (!provider.custom) continue;
        entries.push({
            id,
            name: providerName(id, undefined, provider),
            custom: true,
            api: provider.options?.baseURL,
        });
    }

    const providers = entries
        .filter((entry) => providerMatchesTerm(entry.id, entry.name, term))
        .sort((a, b) => {
            const byRank = providerRank(a.id) - providerRank(b.id);
            return byRank !== 0 ? byRank : a.name.localeCompare(b.name, 'de');
        });
    if (providers.length === 0) {
        const empty = document.createElement('p');
        empty.className = 'settings-hint';
        empty.textContent = term ? 'Keine passenden Anbieter.' : 'Keine Anbieter im Katalog.';
        list.appendChild(empty);
    }
    for (const entry of providers) {
        const card = providerCard(
            entry.id,
            entry.name,
            aiConfig.provider[entry.id]?.enabled === true,
            entry.api,
            entry.doc,
            entry.custom,
        );
        if (entry.custom) {
            card.classList.add('settings-ai-card--custom');
            const actions = document.createElement('div');
            actions.className = 'settings-ai-card__actions';
            const edit = button('Bearbeiten');
            edit.id = `ai-custom-edit-${entry.id}`;
            edit.dataset.aiCustomEdit = entry.id;
            edit.addEventListener('click', () => openCustomDialog(entry.id));
            const remove = button('Löschen');
            remove.id = `ai-custom-delete-${entry.id}`;
            remove.dataset.aiCustomDelete = entry.id;
            remove.addEventListener('click', () => deleteCustomProvider(entry.id));
            actions.append(edit, remove);
            card.appendChild(actions);
        }
        list.appendChild(card);
    }
}

function openCustomDialog(providerId?: string): void {
    const dialog = el('ai-custom-dialog');
    const idInput = input('ai-custom-id');
    const nameInput = input('ai-custom-name');
    const baseUrlInput = input('ai-custom-base-url');
    const keyInput = input('ai-custom-key');
    if (!dialog || !idInput || !nameInput || !baseUrlInput || !keyInput) return;
    const provider = providerId ? aiConfig?.provider[providerId] : undefined;
    const titleEl = el('ai-custom-title');
    if (titleEl) titleEl.textContent = provider ? 'Anbieter bearbeiten' : 'Anbieter hinzufügen';
    idInput.value = providerId || '';
    idInput.disabled = !!provider;
    nameInput.value = provider?.name || '';
    baseUrlInput.value = provider?.options?.baseURL || '';
    keyInput.value = '';
    setError('ai-custom-error', null);
    (dialog as HTMLDivElement).hidden = false;
    (provider ? nameInput : idInput).focus();
}

function closeCustomDialog(): void {
    const dialog = el('ai-custom-dialog') as HTMLDivElement | null;
    const keyInput = input('ai-custom-key');
    if (keyInput) keyInput.value = '';
    if (dialog) dialog.hidden = true;
    setError('ai-custom-error', null);
}

async function saveCustomProvider(event?: Event): Promise<void> {
    if (event) event.preventDefault();
    const idInput = input('ai-custom-id');
    const nameInput = input('ai-custom-name');
    const baseUrlInput = input('ai-custom-base-url');
    const keyInput = input('ai-custom-key');
    if (!idInput || !nameInput || !baseUrlInput || !keyInput) return;
    setError('ai-custom-error', null);
    const definition = {
        id: idInput.value.trim(),
        name: nameInput.value.trim(),
        baseURL: baseUrlInput.value.trim(),
    };
    const result = await invokeUi<AiConfig>(
        'ai_custom_upsert',
        { definition },
        'Custom-Provider konnte nicht gespeichert werden',
    );
    if (result.error) {
        setError('ai-custom-error', result.error);
        return;
    }
    aiConfig = result.value!;

    const key = keyInput.value;
    keyInput.value = '';
    if (key.trim()) {
        const authResult = await invokeUi<Record<string, boolean>>(
            'ai_auth_set',
            { providerId: definition.id, key },
            'Schlüssel des Custom-Providers konnte nicht gespeichert werden',
        );
        await reloadAuthStatus();
        if (authResult.error) {
            setError('ai-custom-error', authResult.error);
            renderProviders();
            return;
        }
    }
    closeCustomDialog();
    renderProviders();
    renderModels();
}

async function deleteCustomProvider(providerId: string): Promise<void> {
    const result = await invokeUi<AiConfig>(
        'ai_custom_delete',
        { id: providerId },
        'Custom-Provider konnte nicht gelöscht werden',
    );
    if (result.error) {
        setError('ai-providers-error', result.error);
        return;
    }
    aiConfig = result.value!;
    renderProviders();
    renderModels();
}

function formatCount(value: number): string {
    if (value >= 1_000_000) return `${Number((value / 1_000_000).toFixed(1))}m`;
    if (value >= 1_000) return `${Number((value / 1_000).toFixed(1))}k`;
    return String(value);
}

function formatCost(value: number): string {
    return Number.isInteger(value) ? String(value) : String(Number(value.toFixed(3)));
}

function modelBadges(model?: CatalogModel): string[] {
    if (!model) return [];
    const badges: string[] = [];
    if (model.limit?.context) badges.push(`Kontext ${formatCount(model.limit.context)}`);
    if (model.reasoning) badges.push('Reasoning');
    if (model.tool_call) badges.push('Tools');
    if (model.cost?.input !== undefined && model.cost.output !== undefined) {
        badges.push(
            `$${formatCost(model.cost.input)}/$${formatCost(model.cost.output)} je 1M`,
        );
    }
    return badges;
}

function configuredModels(
    providerId: string,
    provider: ProviderConfig,
): Array<[string, CatalogModel | ConfiguredModel]> {
    const source: Record<string, CatalogModel | ConfiguredModel> = provider.custom
        ? provider.models || {}
        : catalogResult?.catalog[providerId]?.models || {};
    const ids = new Set([...Object.keys(source), ...(provider.whitelist || [])]);
    const whitelisted = new Set(provider.whitelist || []);
    return Array.from(ids)
        .map((id) => [id, source[id] || {}] as [string, CatalogModel | ConfiguredModel])
        .sort(([idA, a], [idB, b]) => {
            // Verwendete (whitelistete) Modelle zuerst, danach der Rest —
            // jede Gruppe alphabetisch.
            const byUse = (whitelisted.has(idA) ? 0 : 1) - (whitelisted.has(idB) ? 0 : 1);
            if (byUse !== 0) return byUse;
            return (a.name || idA).localeCompare(b.name || idB, 'de');
        });
}

async function toggleModel(providerId: string, modelId: string, on: boolean): Promise<void> {
    const result = await invokeUi<AiConfig>(
        'ai_model_toggle',
        { providerId, modelId, on },
        'KI-Modell konnte nicht geändert werden',
    );
    if (result.error) {
        setError('ai-models-error', result.error);
        renderModels();
        return;
    }
    aiConfig = result.value!;
    setError('ai-models-error', null);
    renderModels();
}

function modelRow(
    providerId: string,
    provider: ProviderConfig,
    modelId: string,
    model: CatalogModel | ConfiguredModel,
): HTMLElement {
    const row = document.createElement('div');
    row.className = 'settings-ai-model';
    row.dataset.aiModelId = modelId;
    const text = document.createElement('div');
    text.className = 'settings-ai-model__text';
    const name = document.createElement('strong');
    name.textContent = model.name || modelId;
    const id = document.createElement('span');
    id.textContent = modelId;
    text.append(name, id);
    const badges = document.createElement('div');
    badges.className = 'settings-ai-model__badges';
    for (const badgeText of modelBadges(provider.custom ? undefined : model as CatalogModel)) {
        const badge = document.createElement('span');
        badge.textContent = badgeText;
        badges.appendChild(badge);
    }
    const toggle = makeToggle(
        `ai-model-toggle-${providerId}-${modelId}`,
        provider.whitelist?.includes(modelId) === true,
        'Verwenden',
        (checked) => toggleModel(providerId, modelId, checked),
    );

    const isWhitelisted = provider.whitelist?.includes(modelId) === true;
    if (isWhitelisted) {
        const actions = document.createElement('div');
        actions.className = 'settings-ai-model__actions';
        const testBtn = button('Test', 'settings-ai-button settings-ai-button--small');
        testBtn.addEventListener('click', (ev) => {
            ev.stopPropagation();
            openChatTest(providerId, modelId);
        });
        actions.append(testBtn, toggle);
        row.append(text, badges, actions);
    } else {
        row.append(text, badges, toggle);
    }
    return row;
}

function renderDefaultModels(): void {
    const element = select('ai-default-model');
    if (!element || !aiConfig) return;
    populateModelPicker(element, aiConfig, catalogResult || { catalog: {} }, {
        includeEmptyOption: true,
        emptyOptionLabel: '(keins)',
        separator: ' — ',
    });
}

function renderModels(): void {
    const list = el('ai-model-list');
    if (!list || !aiConfig || !catalogResult) return;
    list.textContent = '';
    renderDefaultModels();
    const term = (input('ai-model-search')?.value || '').trim().toLocaleLowerCase('de');
    const providers = Object.entries(aiConfig.provider)
        .filter(([, provider]) => provider.enabled)
        .sort(([idA, a], [idB, b]) =>
            providerName(idA, catalogResult!.catalog[idA], a).localeCompare(
                providerName(idB, catalogResult!.catalog[idB], b),
                'de',
            ));
    if (providers.length === 0) {
        const empty = document.createElement('p');
        empty.className = 'settings-ai-empty';
        empty.textContent = 'Aktiviere zuerst Anbieter im Tab KI-Anbieter.';
        list.appendChild(empty);
        return;
    }

    let rendered = 0;
    for (const [providerId, provider] of providers) {
        const name = providerName(
            providerId,
            catalogResult.catalog[providerId],
            provider,
        );
        const providerMatches = `${name} ${providerId}`.toLocaleLowerCase('de').includes(term);
        const models = configuredModels(providerId, provider)
            .filter(([modelId, model]) =>
                !term || providerMatches ||
                `${model.name || ''} ${modelId}`.toLocaleLowerCase('de').includes(term));
        if (term && !providerMatches && models.length === 0) continue;

        const group = document.createElement('section');
        group.className = 'settings-ai-model-group';
        group.dataset.aiModelProvider = providerId;
        const header = document.createElement('div');
        header.className = 'settings-ai-model-group__header';
        const title = document.createElement('h3');
        title.textContent = name;
        header.appendChild(title);
        if (provider.custom) {
            const fetch = button('Modelle abrufen');
            fetch.id = `ai-models-fetch-${providerId}`;
            fetch.dataset.aiModelsFetch = providerId;
            fetch.addEventListener('click', () => fetchCustomModels(providerId, fetch));
            header.appendChild(fetch);
        }
        group.appendChild(header);
        const error = document.createElement('p');
        error.id = `ai-models-fetch-error-${providerId}`;
        error.className = 'settings-ai-error';
        error.hidden = true;
        group.appendChild(error);
        if (models.length === 0) {
            const empty = document.createElement('p');
            empty.className = 'settings-hint';
            empty.textContent = provider.custom
                ? 'Noch keine Modelle. Rufe die Modellliste vom Anbieter ab.'
                : 'Keine passenden Modelle.';
            group.appendChild(empty);
        } else {
            for (const [modelId, model] of models) {
                group.appendChild(modelRow(providerId, provider, modelId, model));
            }
        }
        list.appendChild(group);
        rendered += 1;
    }
    if (rendered === 0) {
        const empty = document.createElement('p');
        empty.className = 'settings-ai-empty';
        empty.textContent = 'Keine passenden Modelle.';
        list.appendChild(empty);
    }
}

async function fetchCustomModels(
    providerId: string,
    fetchButton: HTMLButtonElement,
): Promise<void> {
    const errorId = `ai-models-fetch-error-${providerId}`;
    setError(errorId, null);
    fetchButton.disabled = true;
    fetchButton.textContent = 'Wird abgerufen…';
    const result = await invokeUi<AiConfig>(
        'ai_custom_models_fetch',
        { providerId },
        'Modelle des Custom-Providers konnten nicht abgerufen werden',
    );
    if (result.error) {
        fetchButton.disabled = false;
        fetchButton.textContent = 'Modelle abrufen';
        setError(errorId, result.error);
        return;
    }
    aiConfig = result.value!;
    renderModels();
}

function formatCatalogDate(updatedAt: string): string {
    let date: Date;
    if (/^\d{4}-\d{2}-\d{2}$/.test(updatedAt)) {
        const [year, month, day] = updatedAt.split('-').map(Number);
        date = new Date(year, month - 1, day);
    } else {
        date = new Date(Number(updatedAt) * 1000);
    }
    return Number.isNaN(date.getTime())
        ? updatedAt
        : new Intl.DateTimeFormat('de-DE', { dateStyle: 'medium' }).format(date);
}

function renderCatalogUpdated(): void {
    const element = el('ai-catalog-updated');
    if (!element || !catalogResult) return;
    const source = catalogResult.source === 'cache' ? 'Cache' : 'Snapshot';
    element.textContent = `Katalogstand: ${formatCatalogDate(catalogResult.updatedAt)} (${source})`;
}

async function refreshCatalog(): Promise<void> {
    const refresh = el('ai-catalog-refresh') as HTMLButtonElement | null;
    if (!refresh) return;
    refresh.disabled = true;
    refresh.textContent = 'Wird aktualisiert…';
    setError('ai-models-error', null);
    const result = await invokeUi<CatalogResult>(
        'ai_catalog_refresh',
        undefined,
        'KI-Katalog konnte nicht aktualisiert werden',
    );
    refresh.disabled = false;
    refresh.textContent = 'Anbieter-/Modellkatalog aktualisieren';
    if (result.error) {
        setError('ai-models-error', result.error);
        return;
    }
    catalogResult = result.value!;
    renderCatalogUpdated();
    renderProviders();
    renderModels();
}

async function setDefaultModel(value: string): Promise<void> {
    let providerId: string | null = null;
    let modelId: string | null = null;
    if (value) {
        [providerId, modelId] = JSON.parse(value) as [string, string];
    }
    const result = await invokeUi<AiConfig>(
        'ai_default_model_set',
        { providerId, modelId },
        'Default-Modell konnte nicht gespeichert werden',
    );
    if (result.error) {
        setError('ai-models-error', result.error);
        renderDefaultModels();
        return;
    }
    aiConfig = result.value!;
    setError('ai-models-error', null);
    renderDefaultModels();
}

async function loadAiData(): Promise<void> {
    if (loadPromise) return loadPromise;
    const providerList = el('ai-provider-list');
    const modelList = el('ai-model-list');
    if (providerList) providerList.textContent = 'Wird geladen…';
    if (modelList) modelList.textContent = 'Wird geladen…';
    loadPromise = Promise.all([
        invokeUi<CatalogResult>('ai_catalog_get', undefined, 'KI-Katalog laden'),
        invokeUi<AiConfig>('ai_config_get', undefined, 'KI-Konfiguration laden'),
        invokeUi<Record<string, boolean>>(
            'ai_auth_status',
            undefined,
            'KI-Schlüsselstatus laden',
        ),
    ]).then(([catalog, config, status]) => {
        if (catalog.error || config.error || status.error || !catalog.value || !config.value || !status.value) {
            setError('ai-providers-error', 'KI-Einstellungen konnten nicht geladen werden.');
            setError('ai-models-error', 'KI-Einstellungen konnten nicht geladen werden.');
            return;
        }
        catalogResult = catalog.value!;
        aiConfig = config.value!;
        authStatus = status.value!;
        setError('ai-providers-error', null);
        setError('ai-models-error', null);
        renderCatalogUpdated();
        renderProviders();
        renderModels();
        updateStatusModel();
    }).finally(() => {
        loadPromise = null;
    });
    return loadPromise;
}

export function initSettingsAi(): void {
    if (!el('settings-panel-ki-anbieter') && !el('settings-panel-ki-modelle')) return;
    catalogResult = null;
    aiConfig = null;
    authStatus = {};
    loadPromise = null;
    el('settings-tab-ki-anbieter')?.addEventListener('click', () => loadAiData());
    el('settings-tab-ki-modelle')?.addEventListener('click', () => loadAiData());
    el('ai-custom-add')?.addEventListener('click', () => openCustomDialog());
    el('ai-custom-cancel')?.addEventListener('click', closeCustomDialog);
    const form = el('ai-custom-form');
    if (form) form.addEventListener('submit', (e) => { void saveCustomProvider(e as Event); });
    const dlg = el('ai-custom-dialog');
    if (dlg) dlg.addEventListener('keydown', (event) => {
        const ke = event as KeyboardEvent;
        if (ke.key !== 'Escape') return;
        ke.preventDefault();
        ke.stopPropagation();
        closeCustomDialog();
    });
    input('ai-provider-search')?.addEventListener('input', renderProviders);
    input('ai-model-search')?.addEventListener('input', renderModels);
    el('ai-catalog-refresh')?.addEventListener('click', () => void refreshCatalog());
    const defSel = select('ai-default-model');
    if (defSel) defSel.addEventListener('change', (event) => {
        setDefaultModel((event.currentTarget as HTMLSelectElement).value);
    });
}

// --- Model picker (ported from folio ai-model-picker.ts) ---
export type CatalogModelPicker = { id: string; name?: string };
export type CatalogProviderPicker = {
    id: string;
    name?: string;
    models?: Record<string, CatalogModelPicker>;
};
export type CatalogResultPicker = { catalog: Record<string, CatalogProviderPicker> };

export function populateModelPicker(
    selectElement: HTMLSelectElement,
    config: AiConfig,
    catalog: CatalogResultPicker,
    options: {
        includeEmptyOption?: boolean;
        emptyOptionLabel?: string;
        separator?: string;
    } = {}
): void {
    const includeEmptyOption = options.includeEmptyOption ?? false;
    const emptyOptionLabel = options.emptyOptionLabel ?? '(keins)';
    const separator = options.separator ?? ' · ';

    selectElement.textContent = '';

    if (includeEmptyOption) {
        const empty = document.createElement('option');
        empty.value = '';
        empty.textContent = emptyOptionLabel;
        selectElement.appendChild(empty);
    }

    const choices: Array<{ value: string; label: string }> = [];
    for (const [providerId, provider] of Object.entries(config.provider)) {
        if (!provider.enabled) continue;

        const pName = provider.name || catalog.catalog[providerId]?.name || providerId;

        for (const modelId of new Set(provider.whitelist || [])) {
            const mName = provider.custom
                ? provider.models?.[modelId]?.name || modelId
                : catalog.catalog[providerId]?.models?.[modelId]?.name || modelId;

            choices.push({
                value: JSON.stringify([providerId, modelId]),
                label: `${pName}${separator}${mName}`,
            });
        }
    }

    choices.sort((a, b) => a.label.localeCompare(b.label, 'de'));

    for (const choice of choices) {
        const option = document.createElement('option');
        option.value = choice.value;
        option.textContent = choice.label;
        selectElement.appendChild(option);
    }

    const preferred = config.defaultModel
        ? JSON.stringify([config.defaultModel.provider, config.defaultModel.model])
        : '';

    const hasPreferred = choices.some((choice) => choice.value === preferred);
    if (hasPreferred) {
        selectElement.value = preferred;
    } else {
        selectElement.value = includeEmptyOption ? '' : (choices[0]?.value || '');
    }
}

/// Laedt Katalog und Konfiguration einmalig nach, damit Modell-Auswahlfelder
/// ausserhalb der Einstellungen (z. B. der Zusammenfassen-Dialog) lesbare
/// Anbieter- und Modellnamen anzeigen koennen.
export async function ensureAiData(): Promise<void> {
    if (catalogResult && aiConfig) return;
    await loadAiData();
}

/// Fuellt ein Auswahlfeld mit allen freigeschalteten Modellen aktivierter
/// Anbieter. Vorausgewaehlt ist `preferred`, sofern es noch existiert, sonst
/// das Default-Modell aus den Einstellungen.
export function fillModelPicker(
    selectElement: HTMLSelectElement,
    preferred?: string | null,
): void {
    if (!aiConfig) {
        selectElement.textContent = '';
        return;
    }
    populateModelPicker(selectElement, aiConfig, catalogResult || { catalog: {} }, {
        separator: ' — ',
    });
    if (selectElement.options.length === 0) {
        const empty = document.createElement('option');
        empty.value = '';
        empty.textContent = 'Kein Modell aktiviert - bitte in den Einstellungen wählen';
        selectElement.appendChild(empty);
        return;
    }
    if (!preferred) return;
    const match = Array.from(selectElement.options).some((option) => option.value === preferred);
    if (match) selectElement.value = preferred;
}

// --- Glue for youtube-summarizer: open, apply, chat test (kept as extension) ---

function activateSettingsTab(slug: string) {
  const tabs = document.querySelectorAll<HTMLButtonElement>('[id^="settings-tab-"]');
  tabs.forEach((t) => {
    const active = t.id === `settings-tab-${slug}`;
    t.setAttribute("aria-selected", active ? "true" : "false");
    t.classList.toggle("settings-dialog__tab--active", active);
    t.tabIndex = active ? 0 : -1;
  });
  document.querySelectorAll<HTMLElement>('[role="tabpanel"][data-settings-tab]').forEach((p) => {
    p.hidden = p.dataset.settingsTab !== slug;
  });
}

async function openSettings() {
  try {
    showModal("#settingsModal");
    activateSettingsTab("ki-anbieter");
    await loadAiData();
  } catch (error) {
    setStatusFn(errorMessage(error));
  }
}

export function applyConfig(config: AiConfig) {
  aiConfig = config;
  updateStatusModel();
  const modal = document.getElementById("settingsModal");
  if (modal && !modal.hidden) {
    renderProviders();
    renderModels();
  }
}

function updateStatusModel() {
  if (!statusModelEl || !aiConfig) return;
  const dm = aiConfig.defaultModel;
  if (dm) {
    statusModelEl.textContent = `${dm.provider} / ${dm.model}`;
  } else {
    statusModelEl.textContent = "Kein Modell gewählt";
  }
}

async function openChatTest(pid: string, mid: string) {
  const modal = document.getElementById("chatTestModal") as HTMLDivElement | null;
  if (!modal) { setStatusFn("Chat-Test Dialog nicht vorhanden"); return; }
  const title = document.getElementById("chatTestTitle");
  if (title) title.textContent = `Test ${mid}`;
  const meta = document.getElementById("chatTestMeta");
  if (meta) meta.textContent = `${pid} / ${mid}`;
  showModal("#chatTestModal");
  const send = document.getElementById("chatTestSend");
  const close = document.getElementById("chatTestClose");
  const input = document.getElementById("chatTestMessage") as HTMLTextAreaElement | null;
  const list = document.getElementById("chatTestMessages");
  const errEl = document.getElementById("chatTestError") as HTMLElement | null;
  if (!send || !input || !list) return;
  let msgs: any[] = [];
  const onSend = async () => {
    const m = input.value.trim(); if (!m) return;
    msgs.push({role:"user", content:m}); input.value="";
    renderChat(list as HTMLElement, msgs, true);
    if (errEl) { errEl.textContent = ''; errEl.hidden = true; }
    try {
      const resp = await invoke<string>("ai_model_chat_test", { providerId: pid, modelId: mid, messages: msgs });
      msgs.push({role:"assistant", content: resp});
      renderChat(list as HTMLElement, msgs);
    } catch (e) {
      if (errEl) { errEl.textContent = errorMessage(e); errEl.hidden = false; }
    }
  };
  send.onclick = onSend;
  if (close) close.onclick = () => {
    hideModal("#chatTestModal");
    if (errEl) errEl.hidden = true;
  };
  input.value = "Hi";
  input.focus();
}

function renderChat(list: HTMLElement, msgs: any[], loading=false) {
  list.innerHTML = msgs.map(m => `<div class="chat-message ${m.role}"><span>${m.role}</span><p>${escapeHtml(m.content)}</p></div>`).join("") + (loading ? `<div class="chat-message assistant"><span>Model</span><p>...</p></div>` : "");
  list.scrollTop = list.scrollHeight;
}

// expose for main
export { updateStatusModel };