let bridge = null;
let activeVideoId = null;
let activeTab = 'transcript';
let videos = [];
let activeChapters = null;

function markdownToHtml(md) {
  if (!md) return '';
  var html = md;

  html = html.replace(/([~`]{3,})([\s\S]*?)\1/g, function (m, fence, code) {
    var lang = '';
    var firstNewline = code.indexOf('\n');
    if (firstNewline > -1) {
      var firstLine = code.substring(0, firstNewline).trim();
      if (firstLine && !/[`~]/.test(firstLine)) { lang = firstLine; code = code.substring(firstNewline + 1); }
    }
    return '<pre><code>' + escapeHtml(code.trimEnd()) + '</code></pre>';
  });

  html = html.replace(/`([^`]+)`/g, '<code>$1</code>');

  html = html.replace(/^###### (.+)$/gm, '<h6>$1</h6>');
  html = html.replace(/^##### (.+)$/gm, '<h5>$1</h5>');
  html = html.replace(/^#### (.+)$/gm, '<h4>$1</h4>');
  html = html.replace(/^### (.+)$/gm, '<h3>$1</h3>');
  html = html.replace(/^## (.+)$/gm, '<h2>$1</h2>');
  html = html.replace(/^# (.+)$/gm, '<h1>$1</h1>');

  html = html.replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>');
  html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
  html = html.replace(/(?<!\*)\*(?!\*)(.+?)(?<!\*)\*(?!\*)/g, '<em>$1</em>');
  html = html.replace(/___(.+?)___/g, '<strong><em>$1</em></strong>');
  html = html.replace(/__(.+?)__/g, '<strong>$1</strong>');
  html = html.replace(/(?<!_)_(?!_)(.+?)(?<!_)_(?!_)/g, '<em>$1</em>');

  html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank">$1</a>');

  html = html.replace(/^(\s*)[-*+] (.+)$/gm, function (m, indent, item) {
    return '<li>' + item + '</li>';
  });

  html = html.replace(/^(\s*)\d+\. (.+)$/gm, function (m, indent, item) {
    return '<li>' + item + '</li>';
  });

  html = html.replace(/((?:<li>[\s\S]*?<\/li>\n?)+)/g, function (m) {
    if (m.indexOf('<li>') === -1) return m;
    return '<ul>' + m + '</ul>';
  });

  html = html.replace(/^---+$/gm, '<hr>');

  html = html.replace(/^> (.+)$/gm, '<blockquote>$1</blockquote>');

  var blocks = html.split(/\n\n+/);
  html = blocks.map(function (block) {
    block = block.trim();
    if (!block) return '';
    if (/^<(h[1-6]|ul|ol|pre|blockquote|hr|li)/.test(block)) return block;
    return '<p>' + block.replace(/\n/g, '<br>') + '</p>';
  }).join('\n');

  return html;
}

function renderTranscript(raw, chapters) {
  if (!raw) return '<p class="empty">Kein Transkript verfügbar</p>';
  if (raw.charAt(0) !== '[') return raw.replace(/\n/g, '<br>');

  try {
    var snippets = JSON.parse(raw);
  } catch (e) {
    return raw.replace(/\n/g, '<br>');
  }

  var chapterIdx = 0;
  var prevTime = -1;
  var html = '';

  for (var i = 0; i < snippets.length; i++) {
    var s = snippets[i];
    while (chapters && chapterIdx < chapters.length && chapters[chapterIdx].start <= s.start) {
      var ch = chapters[chapterIdx];
      html += '<div class="ts-chapter" data-start="' + ch.start + '">📌 ' + escapeHtml(ch.title) + '</div>';
      chapterIdx++;
    }
    if (s.time === prevTime) {
      html += ' ' + escapeHtml(s.text);
    } else {
      html += '<div class="ts-line"><span class="ts-time">' + escapeHtml(s.time) + '</span> ' + escapeHtml(s.text) + '</div>';
    }
    prevTime = s.time;
  }

  // remaining chapters beyond transcript
  while (chapters && chapterIdx < chapters.length) {
    var ch = chapters[chapterIdx];
    html += '<div class="ts-chapter">📌 ' + escapeHtml(ch.title) + '</div>';
    chapterIdx++;
  }

  return html;
}

function renderChapters(chapters) {
  var list = document.getElementById('chaptersList');
  var panel = document.getElementById('chaptersPanel');
  if (!chapters || !chapters.length) {
    panel.style.display = 'none';
    return;
  }
  panel.style.display = 'block';
  var html = '';
  chapters.forEach(function (c) {
    html += '<div class="chapter-item" data-start="' + c.start + '"><span class="ts-time">' + escapeHtml(c.time) + '</span> ' + escapeHtml(c.title) + '</div>';
  });
  list.innerHTML = html;

  list.querySelectorAll('.chapter-item').forEach(function (item) {
    item.addEventListener('click', function () {
      seekVideo(parseFloat(this.dataset.start));
    });
  });
}

function seekVideo(seconds) {
  var iframe = document.getElementById('videoPlayer');
  var v = findVideo(activeVideoId);
  if (!v) return;
  iframe.src = 'https://www.youtube-nocookie.com/embed/' + v.video_id + '?start=' + Math.floor(seconds) + '&autoplay=1';
  switchTab('video');
}

function buildSummaryPrompt() {
  var detail = document.getElementById('summaryDetail').value;
  var lang = document.getElementById('summaryLang').value;
  var useChapters = document.getElementById('summaryUseChapters').value;

  var prompt = 'You are a helpful assistant that summarizes YouTube video transcripts.\n\n';

  if (detail === 'short') {
    prompt += 'Provide a very concise summary: just 3-5 bullet points with the key takeaways.\n';
  } else if (detail === 'detailed') {
    prompt += 'Provide a comprehensive and detailed summary. Include:\n';
    prompt += '- A short overview (2-3 sentences)\n';
    prompt += '- All main topics discussed, organized by theme\n';
    prompt += '- Key arguments, facts, and insights\n';
    prompt += '- Main conclusions and takeaways\n';
  } else {
    prompt += 'Provide a clear, structured summary. Include:\n';
    prompt += '- A short overview (1-2 sentences)\n';
    prompt += '- Key points as bullet points\n';
    prompt += '- Main conclusions or takeaways\n';
  }

  var langNames = {
    'original': 'the same language as the transcript',
    'german': 'German', 'english': 'English', 'french': 'French',
    'spanish': 'Spanish', 'italian': 'Italian'
  };
  prompt += '\nWrite the summary in ' + langNames[lang] + '.\n';

  if (useChapters === 'yes') {
    prompt += '\nThe transcript may contain chapter markers. If present, structure the summary by chapter.\n';
  }

  prompt += '\nFormat your response as Markdown.';
  return prompt;
}

function updateSummaryPrompt() {
  document.getElementById('summaryPrompt').value = buildSummaryPrompt();
}

function openSummaryDialog() {
  if (activeVideoId === null) return;
  var v = findVideo(activeVideoId);
  if (!v) return;
  if (!v.transcript) {
    setStatus('Kein Transkript vorhanden – bitte warten oder Video neu hinzufügen');
    return;
  }
  updateSummaryPrompt();
  document.getElementById('summaryModal').style.display = 'flex';
}

function startSummary() {
  var prompt = document.getElementById('summaryPrompt').value.trim();
  if (activeVideoId !== null) {
    bridge.summarize_video(activeVideoId, prompt);
    document.getElementById('summaryModal').style.display = 'none';
  }
}

function findVideo(id) {
  for (var i = 0; i < videos.length; i++) {
    if (videos[i].id === id) return videos[i];
  }
  return null;
}

function init() {
  new QWebChannel(qt.webChannelTransport, function (channel) {
    bridge = channel.objects.bridge;

    bridge.videos_loaded.connect(function (json) {
      videos = JSON.parse(json);
      renderVideoList();
    });

    bridge.video_detail_loaded.connect(function (json) {
      var v = JSON.parse(json);
      activeChapters = v.chapters;
      showDetail(v);
    });

    bridge.transcript_loaded.connect(function (dbId, transcript) {
      videos.forEach(function (v) { if (v.id === dbId) v.transcript = transcript; });
      renderVideoList();
      if (activeVideoId === dbId) {
        document.getElementById('tabTranscript').innerHTML = renderTranscript(transcript, activeChapters);
      }
    });

    bridge.chapters_loaded.connect(function (dbId, chaptersJson) {
      activeChapters = JSON.parse(chaptersJson);
      videos.forEach(function (v) { if (v.id === dbId) v.chapters = activeChapters; });
      if (activeVideoId === dbId) {
        renderChapters(activeChapters);
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
      var v = JSON.parse(json);
      videos.unshift(v);
      renderVideoList();
      selectVideo(v.id);
    });

    bridge.video_deleted.connect(function (dbId) {
      videos = videos.filter(function (v) { return v.id !== dbId; });
      renderVideoList();
      if (activeVideoId === dbId) {
        activeVideoId = null;
        activeChapters = null;
        document.getElementById('detailContent').style.display = 'none';
        document.getElementById('detailPlaceholder').style.display = 'flex';
      }
    });

    bridge.error.connect(function (msg) {
      setStatus(msg);
    });

    bridge.config_loaded.connect(function (json) {
      var cfg = JSON.parse(json);
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
    bridge.save_config(
      document.getElementById('configProvider').value,
      document.getElementById('configApiKey').value,
      document.getElementById('configModel').value
    );
    document.getElementById('settingsModal').style.display = 'none';
  });

  document.getElementById('configCancel').addEventListener('click', function () {
    document.getElementById('settingsModal').style.display = 'none';
  });

  document.getElementById('summarizeBtn').addEventListener('click', openSummaryDialog);

  document.getElementById('summaryStart').addEventListener('click', startSummary);
  document.getElementById('summaryCancel').addEventListener('click', function () {
    document.getElementById('summaryModal').style.display = 'none';
  });

  document.getElementById('summaryDetail').addEventListener('change', updateSummaryPrompt);
  document.getElementById('summaryLang').addEventListener('change', updateSummaryPrompt);
  document.getElementById('summaryUseChapters').addEventListener('change', updateSummaryPrompt);

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

  document.getElementById('tabTranscript').addEventListener('click', function (e) {
    var chapter = e.target.closest('.ts-chapter');
    if (chapter) {
      var secs = parseFloat(chapter.dataset.start);
      if (!isNaN(secs)) seekVideo(secs);
    }
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
  document.getElementById('tabTranscript').innerHTML = renderTranscript(v.transcript, v.chapters);
  document.getElementById('tabSummary').innerHTML = markdownToHtml(v.summary || '');
  document.getElementById('videoPlayer').src = 'https://www.youtube-nocookie.com/embed/' + v.video_id;
  activeChapters = v.chapters;
  renderChapters(v.chapters);
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
