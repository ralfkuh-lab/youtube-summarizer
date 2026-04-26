let bridge = null;
let activeVideoId = null;
let activeTab = 'transcript';
let videos = [];

function markdownToHtml(md) {
  if (!md) return '';
  var html = md;

  // code blocks (``` or ~~~)
  html = html.replace(/([~`]{3,})([\s\S]*?)\1/g, function (m, fence, code) {
    var lang = '';
    var firstNewline = code.indexOf('\n');
    if (firstNewline > -1) {
      var firstLine = code.substring(0, firstNewline).trim();
      if (firstLine && !/[`~]/.test(firstLine)) { lang = firstLine; code = code.substring(firstNewline + 1); }
    }
    return '<pre><code>' + escapeHtml(code.trimEnd()) + '</code></pre>';
  });

  // inline code
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>');

  // headings
  html = html.replace(/^###### (.+)$/gm, '<h6>$1</h6>');
  html = html.replace(/^##### (.+)$/gm, '<h5>$1</h5>');
  html = html.replace(/^#### (.+)$/gm, '<h4>$1</h4>');
  html = html.replace(/^### (.+)$/gm, '<h3>$1</h3>');
  html = html.replace(/^## (.+)$/gm, '<h2>$1</h2>');
  html = html.replace(/^# (.+)$/gm, '<h1>$1</h1>');

  // bold and italic
  html = html.replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>');
  html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)/g, '<em>$1</em>');
  html = html.replace(/___(.+?)___/g, '<strong><em>$1</em></strong>');
  html = html.replace(/__(.+?)__/g, '<strong>$1</strong>');
  html = html.replace(/(?<!_)_(?!_)(.+?)(?<!_)_(?!_)/g, '<em>$1</em>');

  // links
  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank">$1</a>');

  // unordered lists
  html = html.replace(/^(\s*)[-*+] (.+)$/gm, function (m, indent, item) {
    return '<li>' + item + '</li>';
  });

  // ordered lists
  html = html.replace(/^(\s*)\d+\. (.+)$/gm, function (m, indent, item) {
    return '<li>' + item + '</li>';
  });

  // wrap consecutive li elements in ul/ol
  html = html.replace(/((?:<li>[\s\S]*?<\/li>\n?)+)/g, function (m) {
    if (m.indexOf('<li>') === -1) return m;
    return '<ul>' + m + '</ul>';
  });

  // horizontal rule
  html = html.replace(/^---+$/gm, '<hr>');

  // blockquote
  html = html.replace(/^> (.+)$/gm, '<blockquote>$1</blockquote>');

  // paragraphs: split by double newlines
  var blocks = html.split(/\n\n+/);
  html = blocks.map(function (block) {
    block = block.trim();
    if (!block) return '';
    if (/^<(h[1-6]|ul|ol|pre|blockquote|hr|li)/.test(block)) return block;
    return '<p>' + block.replace(/\n/g, '<br>') + '</p>';
  }).join('\n');

  return html;
}

function init() {
  new QWebChannel(qt.webChannelTransport, function (channel) {
    bridge = channel.objects.bridge;

    bridge.videos_loaded.connect(function (json) {
      videos = JSON.parse(json);
      renderVideoList();
    });

    bridge.video_detail_loaded.connect(function (json) {
      const v = JSON.parse(json);
      showDetail(v);
    });

    bridge.transcript_loaded.connect(function (dbId, transcript) {
      videos.forEach(function (v) { if (v.id === dbId) v.transcript = transcript; });
      renderVideoList();
      if (activeVideoId === dbId) {
        document.getElementById('tabTranscript').textContent = transcript;
      }
    });

    bridge.summary_loaded.connect(function (dbId, summary) {
      videos.forEach(function (v) { if (v.id === dbId) v.summary = summary; });
      renderVideoList();
      if (activeVideoId === dbId) {
        document.getElementById('tabSummary').innerHTML = markdownToHtml(summary);
        switchTab('summary');
      }
    });

    bridge.video_added.connect(function (json) {
      const v = JSON.parse(json);
      videos.unshift(v);
      renderVideoList();
      selectVideo(v.id);
    });

    bridge.video_deleted.connect(function (dbId) {
      videos = videos.filter(function (v) { return v.id !== dbId; });
      renderVideoList();
      if (activeVideoId === dbId) {
        activeVideoId = null;
        document.getElementById('detailContent').style.display = 'none';
        document.getElementById('detailPlaceholder').style.display = 'flex';
      }
    });

    bridge.error.connect(function (msg) {
      setStatus(msg);
    });

    bridge.config_loaded.connect(function (json) {
      const cfg = JSON.parse(json);
      document.getElementById('configProvider').value = cfg.provider;
      document.getElementById('configApiKey').value = cfg.api_key;
      document.getElementById('configModel').value = cfg.model;
      document.getElementById('statusModel').textContent = 'Provider: ' + cfg.provider + ' / Modell: ' + cfg.model;
    });

    bridge.status_update.connect(function (msg) {
      setStatus(msg);
    });

    bridge.get_videos();
    bridge.get_config();
  });

  document.getElementById('addBtn').addEventListener('click', addVideo);
  document.getElementById('urlInput').addEventListener('keydown', function (e) {
    if (e.key === 'Enter') addVideo();
  });

  document.getElementById('settingsBtn').addEventListener('click', function () {
    bridge.get_config();
    document.getElementById('settingsModal').style.display = 'flex';
  });

  document.getElementById('configSave').addEventListener('click', function () {
    var provider = document.getElementById('configProvider').value;
    var apiKey = document.getElementById('configApiKey').value;
    var model = document.getElementById('configModel').value;
    bridge.save_config(provider, apiKey, model);
    document.getElementById('settingsModal').style.display = 'none';
  });

  document.getElementById('configCancel').addEventListener('click', function () {
    document.getElementById('settingsModal').style.display = 'none';
  });

  document.getElementById('summarizeBtn').addEventListener('click', function () {
    if (activeVideoId !== null) bridge.summarize_video(activeVideoId);
  });

  document.getElementById('deleteBtn').addEventListener('click', function () {
    if (activeVideoId !== null && confirm('Video wirklich löschen?')) {
      bridge.delete_video(activeVideoId);
    }
  });

  document.querySelectorAll('.tab').forEach(function (tab) {
    tab.addEventListener('click', function () {
      switchTab(this.dataset.tab);
    });
  });
}

function addVideo() {
  var input = document.getElementById('urlInput');
  var url = input.value.trim();
  if (!url) return;
  bridge.add_video(url);
  input.value = '';
  setStatus('Video wird hinzugefügt...');
}

function renderVideoList() {
  var container = document.getElementById('videoList');
  container.innerHTML = '';
  videos.forEach(function (v) {
    var hasTranscript = v.transcript ? '• T' : '';
    var hasSummary = v.summary ? '• Z' : '';
    var div = document.createElement('div');
    div.className = 'video-item' + (activeVideoId === v.id ? ' active' : '');
    var thumbSrc = v.thumbnail || v.thumbnail_url;
    div.innerHTML =
      '<img src="' + escapeHtml(thumbSrc) + '" alt="" loading="lazy" />' +
      '<div class="info">' +
        '<div class="title">' + escapeHtml(v.title) + '</div>' +
        '<div class="meta"><span>' + escapeHtml(hasTranscript) + '</span><span>' + escapeHtml(hasSummary) + '</span></div>' +
      '</div>';
    div.addEventListener('click', function () { selectVideo(v.id); });
    container.appendChild(div);
  });
}

function selectVideo(id) {
  activeVideoId = id;
  renderVideoList();
  bridge.get_video_detail(id);
}

function showDetail(v) {
  document.getElementById('detailPlaceholder').style.display = 'none';
  document.getElementById('detailContent').style.display = 'flex';
  document.getElementById('detailThumb').src = v.thumbnail || v.thumbnail_url;
  document.getElementById('detailTitle').textContent = v.title;
  document.getElementById('detailUrl').href = v.url;
  document.getElementById('detailUrl').textContent = v.url;
  document.getElementById('tabTranscript').textContent = v.transcript || 'Kein Transkript verfügbar';
  document.getElementById('tabSummary').innerHTML = markdownToHtml(v.summary || '');
  switchTab(activeTab);
}

function switchTab(tab) {
  activeTab = tab;
  document.querySelectorAll('.tab').forEach(function (t) {
    t.classList.toggle('active', t.dataset.tab === tab);
  });
  document.querySelectorAll('.tab-panel').forEach(function (p) {
    p.classList.toggle('active', p.id === 'tab' + capitalize(tab));
  });
}

function setStatus(msg) {
  document.getElementById('statusText').textContent = msg;
}

function capitalize(s) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

function escapeHtml(str) {
  return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
}

window.addEventListener('DOMContentLoaded', init);
