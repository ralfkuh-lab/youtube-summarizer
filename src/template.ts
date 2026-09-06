export const appTemplate = `
  <header>
    <div class="app-brand">
      <h1>YouTube Summarizer</h1>
    </div>
    <div id="addBar">
      <input id="urlInput" type="text" placeholder="YouTube-URL oder Video-ID eingeben..." />
      <button id="addBtn">Hinzufügen</button>
    </div>
    <div class="toolbar-actions">
      <button id="settingsBtn" class="icon-btn" title="Einstellungen" aria-label="Einstellungen">⚙</button>
    </div>
  </header>

  <main>
    <aside id="libraryPanel">
      <div class="collection-tools">
        <div class="library-section-title">
          <span>Sammlungen</span>
          <button id="addCollectionBtn" class="mini-icon-btn" title="Sammlung erstellen" aria-label="Sammlung erstellen">+</button>
        </div>
        <div id="collectionList"></div>
      </div>
      <div class="library-tools">
        <div class="library-search">
          <input id="videoSearchInput" type="search" placeholder="Videos suchen..." autocomplete="off" />
        </div>
        <div class="library-filters" aria-label="Videofilter">
          <button class="filter-chip active" data-video-filter="all">Alle</button>
          <button class="filter-chip" data-video-filter="transcript">Transkript</button>
          <button class="filter-chip" data-video-filter="missing-transcript">Ohne T</button>
          <button class="filter-chip" data-video-filter="summary">Zusammenfassung</button>
          <button class="filter-chip" data-video-filter="missing-summary">Ohne Z</button>
        </div>
      </div>
      <div id="videoList"></div>
    </aside>
    <section id="detail">
      <div id="detailPlaceholder">Wähle ein Video aus der Liste</div>
      <div id="detailContent" hidden>
        <div id="detailHeader">
          <img id="detailThumb" alt="" />
          <div class="detail-title-block">
            <h2 id="detailTitle"></h2>
            <a id="detailUrl" href="#" target="_blank" rel="noreferrer"></a>
            <span id="detailPublishedMeta" class="detail-summary-meta" hidden></span>
            <span id="detailSummaryMeta" class="detail-summary-meta" hidden></span>
            <details id="detailDescription" class="detail-description" hidden>
              <summary>Beschreibung</summary>
              <div id="detailDescriptionText"></div>
            </details>
          </div>
          <button id="deleteBtn" class="delete-icon-btn" title="Video entfernen" aria-label="Video entfernen">🗑</button>
        </div>

        <div id="collectionAssignment" class="collection-assignment"></div>

        <div id="tabBar">
          <button class="tab active" data-tab="transcript">Transkript</button>
          <button class="tab" data-tab="summary">Zusammenfassung</button>
          <button class="tab" data-tab="video">Video</button>
          <button id="reloadTranscriptBtn">Transkript laden</button>
          <button id="summarizeBtn">Zusammenfassen lassen</button>
        </div>

        <div id="tabContent">
          <div id="tabTranscript" class="tabPanel active"></div>
          <div id="tabSummary" class="tabPanel">
            <div id="summaryHistoryBar" class="summary-history-bar" hidden>
              <select id="summaryHistorySelect" aria-label="Zusammenfassungs-Verlauf"></select>
              <button id="summaryHistoryDelete" class="delete-icon-btn" title="Diese Version löschen" aria-label="Diese Version löschen">🗑</button>
            </div>
            <div id="summaryBody"></div>
          </div>
          <div id="tabVideo" class="tabPanel">
            <div id="videoCodecNotice" class="video-codec-notice" hidden></div>
            <div class="video-player-shell">
              <iframe
                id="videoPlayer"
                title="YouTube Video"
                allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share; fullscreen"
                allowfullscreen
                referrerpolicy="strict-origin-when-cross-origin"
              ></iframe>
            </div>
            <div class="video-fallback">
              <a id="videoFallbackLink" href="#" target="_blank" rel="noreferrer">Video auf YouTube öffnen</a>
            </div>
          </div>
        </div>
      </div>

    </section>
    <aside id="chaptersPanel" class="chapters-panel" hidden>
      <h3>Kapitel</h3>
      <div id="chaptersList"></div>
    </aside>
  </main>

  <footer>
    <span id="statusText">Bereit</span>
    <span id="statusModel"></span>
  </footer>

  <div id="settingsModal" class="modal" hidden>
    <div class="modal-content settings-content">
      <div class="settings-dialog__panel">
        <div class="settings-dialog__tabs" role="tablist" aria-label="KI Einstellungen">
          <button id="settings-tab-ki-anbieter" role="tab" aria-selected="true" aria-controls="settings-panel-ki-anbieter" tabindex="0" class="settings-dialog__tab settings-dialog__tab--active">KI-Anbieter</button>
          <button id="settings-tab-ki-modelle" role="tab" aria-selected="false" aria-controls="settings-panel-ki-modelle" tabindex="-1" class="settings-dialog__tab">KI-Modelle</button>
        </div>
        <div class="settings-dialog__tabpanel settings-ai-panel" id="settings-panel-ki-anbieter" role="tabpanel" aria-labelledby="settings-tab-ki-anbieter" data-settings-tab="ki-anbieter" hidden>
          <section class="settings-section">
            <h3 class="settings-section__title">KI-Anbieter</h3>
            <p class="settings-hint">Schlüssel liegen im Klartext in <code>auth.json</code> im Config-Verzeichnis (Dateirechte 0600).</p>
            <div class="settings-ai-toolbar">
              <input type="search" id="ai-provider-search" class="settings-input" placeholder="Anbieter suchen…" autocomplete="off" />
              <button type="button" id="ai-custom-add" class="settings-ai-button">Anbieter hinzufügen</button>
            </div>
            <p class="settings-hint">Die vordefinierten Anbieter stammen aus dem <code>models.dev</code>-Katalog; aktualisieren lässt er sich im Reiter „KI-Modelle“. Eigene (OpenAI-kompatible) Anbieter lassen sich über „Anbieter hinzufügen“ ergänzen.</p>
            <p id="ai-providers-error" class="settings-ai-error" hidden></p>
            <div id="ai-provider-list" class="settings-ai-list" aria-live="polite"></div>
          </section>
          <div id="ai-custom-dialog" class="settings-ai-overlay" hidden>
            <form id="ai-custom-form" class="settings-ai-dialog" role="dialog" aria-modal="true" aria-labelledby="ai-custom-title">
              <h3 id="ai-custom-title">Anbieter hinzufügen</h3>
              <label for="ai-custom-id">ID</label>
              <input type="text" id="ai-custom-id" class="settings-input" autocomplete="off" spellcheck="false" />
              <p class="settings-hint">Nur Kleinbuchstaben, Zahlen, - und _</p>
              <label for="ai-custom-name">Anzeigename</label>
              <input type="text" id="ai-custom-name" class="settings-input" autocomplete="off" />
              <label for="ai-custom-base-url">Basis-URL</label>
              <input type="url" id="ai-custom-base-url" class="settings-input" placeholder="http://localhost:11434/v1" autocomplete="off" spellcheck="false" />
              <p class="settings-hint">OpenAI-kompatibler Endpoint.</p>
              <label for="ai-custom-key">Schlüssel (optional)</label>
              <input type="password" id="ai-custom-key" class="settings-input" autocomplete="new-password" />
              <p id="ai-custom-error" class="settings-ai-error" hidden></p>
              <div class="settings-ai-dialog__actions">
                <button type="button" id="ai-custom-cancel">Abbrechen</button>
                <button type="submit" id="ai-custom-save" class="primary">Speichern</button>
              </div>
            </form>
          </div>
        </div>
        <div class="settings-dialog__tabpanel settings-ai-panel" id="settings-panel-ki-modelle" role="tabpanel" aria-labelledby="settings-tab-ki-modelle" data-settings-tab="ki-modelle" hidden>
          <section class="settings-section">
            <div class="settings-ai-toolbar">
              <input type="search" id="ai-model-search" class="settings-input" placeholder="Provider oder Modell suchen…" autocomplete="off" />
              <button type="button" id="ai-catalog-refresh" class="settings-ai-button" title="Lädt den Anbieter- und Modellkatalog der vordefinierten Cloud-Provider neu von models.dev.">Anbieter-/Modellkatalog aktualisieren</button>
            </div>
            <p class="settings-hint">Lädt Anbieter- und Modellliste der vordefinierten Cloud-Provider von <code>models.dev</code>. Modelle eigener Anbieter holst du im jeweiligen Anbieter über „Modelle abrufen“.</p>
            <p id="ai-catalog-updated" class="settings-hint"></p>
            <p id="ai-models-error" class="settings-ai-error" hidden></p>
            <div class="settings-row">
              <label for="ai-default-model">Default-Modell</label>
              <select id="ai-default-model" class="settings-input">
                <option value="">(keins)</option>
              </select>
            </div>
          </section>
          <section class="settings-section">
            <div id="ai-model-list" class="settings-ai-model-list" aria-live="polite"></div>
          </section>
        </div>
      </div>
      <div class="modal-actions">
        <button id="configClose">Schließen</button>
      </div>
    </div>
  </div>

  <div id="summaryModal" class="modal" hidden>
    <div class="modal-content modal-wide">
      <h2>Zusammenfassung konfigurieren</h2>
      <label>Modell
        <select id="summaryModel"></select>
      </label>
      <label>Vorlage
        <select id="summaryPreset"></select>
      </label>
      <div class="summary-row">
        <label>Detailgrad
          <select id="summaryDetail">
            <option value="short">Kurz</option>
            <option value="medium" selected>Mittel</option>
            <option value="detailed">Ausführlich</option>
          </select>
        </label>
        <label>Sprache
          <select id="summaryLang">
            <option value="original">Original</option>
            <option value="german">Deutsch</option>
            <option value="english">English</option>
            <option value="french">Français</option>
            <option value="spanish">Español</option>
            <option value="italian">Italiano</option>
          </select>
        </label>
        <label>Kapitel nutzen
          <select id="summaryUseChapters">
            <option value="yes">Ja</option>
            <option value="no">Nein</option>
          </select>
        </label>
      </div>
      <fieldset class="summary-modules">
        <legend>Zusätzlich</legend>
        <label class="summary-module">
          <input type="checkbox" id="summaryModTables" checked />
          Tabellen für Daten/Vergleiche
        </label>
        <label class="summary-module">
          <input type="checkbox" id="summaryModMermaid" />
          Mermaid-Diagramme für komplexe Zusammenhänge
        </label>
        <label class="summary-module">
          <input type="checkbox" id="summaryModAssessment" />
          Einordnung durch die KI (Fakt vs. Meinung)
        </label>
        <label class="summary-module">
          <input type="checkbox" id="summaryModVerify" />
          Aussagen kritisch prüfen
        </label>
        <label class="summary-module">
          <input type="checkbox" id="summaryModTimestamps" />
          Timestamps [mm:ss] zu den Abschnitten
        </label>
        <label class="summary-module">
          <input type="checkbox" id="summaryModLinks" />
          Hilfreiche Links aus der Beschreibung als „Ressourcen"
        </label>
      </fieldset>
      <details id="summaryPromptDetails" class="summary-prompt-details">
        <summary>
          <span>Prompt (Vorschau/bearbeiten)</span>
          <span id="summaryPromptEdited" class="summary-edited" hidden>
            Bearbeitet <a href="#" id="summaryPromptReset">Zurücksetzen</a>
          </span>
        </summary>
        <textarea id="summaryPrompt" rows="8"></textarea>
      </details>
      <div class="modal-actions">
        <button id="summaryStart">Zusammenfassen</button>
        <button id="summaryCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="collectionModal" class="modal" hidden>
    <div class="modal-content collection-modal-content">
      <h2 id="collectionModalTitle">Sammlung</h2>
      <label>Name
        <input id="collectionNameInput" type="text" maxlength="80" />
      </label>
      <div class="modal-actions">
        <button id="collectionSave">Speichern</button>
        <button id="collectionCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="presetManageModal" class="modal" hidden>
    <div class="modal-content modal-wide">
      <h2>Vorlagen verwalten</h2>
      <div id="presetList" class="preset-list"></div>
      <div class="modal-actions">
        <button id="presetNew">Neu</button>
        <button id="presetManageClose">Schließen</button>
      </div>
    </div>
  </div>

  <div id="presetEditModal" class="modal" hidden>
    <div class="modal-content">
      <h2 id="presetEditTitle">Vorlage</h2>
      <label id="presetEditIdLabel">ID
        <input id="presetEditId" type="text" maxlength="32" autocomplete="off" spellcheck="false" />
      </label>
      <p id="presetEditIdHint" class="settings-hint">Kleinbuchstaben, Ziffern und Bindestrich, max. 32 Zeichen.</p>
      <label id="presetEditNameLabel">Name
        <input id="presetEditName" type="text" maxlength="80" autocomplete="off" />
      </label>
      <label id="presetEditPromptLabel">Prompt
        <textarea id="presetEditPrompt" rows="8"></textarea>
      </label>
      <p id="presetEditError" class="settings-ai-error" hidden></p>
      <div class="modal-actions">
        <button id="presetEditSave">Speichern</button>
        <button id="presetEditCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="confirmModal" class="modal" hidden>
    <div class="modal-content confirm-content">
      <h2 id="confirmTitle">Bestätigen</h2>
      <p id="confirmMessage"></p>
      <div class="modal-actions">
        <button id="confirmOk">OK</button>
        <button id="confirmCancel">Abbrechen</button>
      </div>
    </div>
  </div>

  <div id="chatTestModal" class="modal" hidden>
    <div class="modal-content chat-test-content">
      <div class="chat-test-head">
        <div>
          <h2 id="chatTestTitle">Test chat</h2>
          <p id="chatTestMeta"></p>
        </div>
      </div>
      <div id="chatTestMessages" class="chat-test-messages"></div>
      <div id="chatTestError" class="chat-test-error" hidden></div>
      <div class="chat-test-composer">
        <textarea id="chatTestMessage" rows="3">Say "ok" in one short sentence.</textarea>
        <button id="chatTestSend">Send</button>
      </div>
      <div class="modal-actions">
        <button id="chatTestClose">Close</button>
      </div>
    </div>
  </div>
`;
