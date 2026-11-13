const MIME_EXT = {
  "svg+xml": "svg",
  jpeg: "jpg",
  pjpeg: "jpg",
  "x-icon": "ico",
  "vnd.microsoft.icon": "ico",
  "quicktime": "mov",
  "x-matroska": "mkv",
};
const MIN_SIZE_KEY = "minSize";
const RESTRICTED_RE =
  /^(about|moz-extension|chrome|resource|view-source|data|file|javascript|blob):|^https?:\/\/(addons\.mozilla\.org|addons\.allizom\.org)(\/|$)/i;

function sanitizeName(name) {
  return name
    .replace(/[\\/:*?"<>|\x00-\x1f]/g, "_")
    .replace(/^\.+/, "")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 100);
}

// Devine un nom de fichier propre ; `index` donne un repli stable (media_N).
function getSmartFilename(url, type, index) {
  const defaultExt = type === "video" ? "mp4" : "jpg";
  const fallback = (ext) => `media_${index + 1}.${ext}`;

  if (url.startsWith("data:")) {
    const m = /^data:([^;,]*)/i.exec(url);
    let sub = m && m[1].includes("/") ? m[1].split("/")[1].toLowerCase() : "";
    const ext = sanitizeName(MIME_EXT[sub] || sub || defaultExt) || defaultExt;
    return fallback(ext);
  }

  try {
    const urlObj = new URL(url);
    let filename = "";
    try {
      filename = decodeURIComponent(urlObj.pathname.split("/").pop());
    } catch (e) {
      filename = urlObj.pathname.split("/").pop();
    }
    filename = sanitizeName(filename);
    if (!filename) return fallback(defaultExt);

    if (!/\.[A-Za-z0-9]{1,5}$/.test(filename)) {
      const fmt = sanitizeName(urlObj.searchParams.get("format") || "")
        .replace(/[^A-Za-z0-9]/g, "")
        .toLowerCase();
      const ext = MIME_EXT[fmt] || fmt || defaultExt;
      filename = `${filename}.${ext}`;
    }
    return filename;
  } catch (e) {
    return fallback(defaultExt);
  }
}

// Noms uniques pour toute une liste (suffixe -1, -2 en cas de collision).
function uniqueNames(medias) {
  const used = new Set();
  return medias.map((m, i) => {
    const base = getSmartFilename(m.src, m.type, i);
    const dot = base.lastIndexOf(".");
    const stem = dot > 0 ? base.slice(0, dot) : base;
    const ext = dot > 0 ? base.slice(dot) : "";
    let name = base;
    let n = 0;
    while (used.has(name.toLowerCase())) {
      n += 1;
      name = `${stem}-${n}${ext}`;
    }
    used.add(name.toLowerCase());
    return name;
  });
}

const errText = (e) => (e && e.message ? e.message : String(e));

document.addEventListener("DOMContentLoaded", () => {
  const gallery = document.getElementById("gallery");
  const titleText = document.getElementById("title-text");
  const btnDownloadAll = document.getElementById("download-all");
  const btnRefresh = document.getElementById("refresh");
  const toolbar = document.getElementById("toolbar");
  const sortSelect = document.getElementById("sort-select");
  const minSizeInput = document.getElementById("min-size");
  const statusEl = document.getElementById("status");
  const noticeEl = document.getElementById("notice");

  let allMedias = [];
  let currentMedias = [];
  let busy = false;

  function setStatus(text, isError) {
    statusEl.hidden = !text;
    statusEl.textContent = text || "";
    statusEl.className = isError ? "error" : "";
  }

  function setNotice(text) {
    noticeEl.hidden = !text;
    noticeEl.textContent = text || "";
  }

  function showMessage(text) {
    gallery.innerHTML = "";
    const div = document.createElement("div");
    div.className = "message";
    div.textContent = text;
    gallery.appendChild(div);
  }

  function updateDownloadAll() {
    const n = currentMedias.length;
    btnDownloadAll.hidden = allMedias.length === 0;
    btnDownloadAll.disabled = busy || n === 0;
    if (!busy) btnDownloadAll.textContent = `Tout télécharger (${n})`;
  }

  function renderGallery(medias) {
    gallery.innerHTML = "";
    gallery.removeAttribute("aria-busy");

    if (medias.length === 0) {
      showMessage("Aucun média ne correspond à ce filtre. Baissez la taille minimale.");
      return;
    }

    medias.forEach((mediaData, idx) => {
      const container = document.createElement("div");
      container.className = "img-container";

      const mediaEl = document.createElement(
        mediaData.type === "video" ? "video" : "img",
      );
      if (mediaData.type === "video") {
        mediaEl.muted = true;
        mediaEl.autoplay = true;
        mediaEl.loop = true;
      } else {
        mediaEl.loading = "lazy";
        mediaEl.alt = `Image ${idx + 1}`;
      }
      mediaEl.addEventListener("error", () => {
        container.classList.add("broken");
        const ph = document.createElement("div");
        ph.className = "broken-msg";
        ph.textContent = "Aperçu indisponible";
        mediaEl.replaceWith(ph);
      });
      mediaEl.src = mediaData.src;

      const badge = document.createElement("div");
      badge.className = "resolution-badge";
      badge.textContent =
        mediaData.width > 0 && mediaData.height > 0
          ? `${mediaData.width} × ${mediaData.height}`
          : "Dimensions inconnues";

      if (mediaData.type === "video") {
        const typeBadge = document.createElement("div");
        typeBadge.className = "type-badge";
        typeBadge.textContent = "VIDÉO";
        container.appendChild(typeBadge);
      }

      const overlay = document.createElement("div");
      overlay.className = "overlay";

      const btnDl = document.createElement("button");
      btnDl.type = "button";
      btnDl.className = "action-btn primary";
      btnDl.textContent = "Télécharger (Choix)";
      btnDl.onclick = async () => {
        try {
          await browser.downloads.download({
            url: mediaData.src,
            filename: getSmartFilename(mediaData.src, mediaData.type, idx),
            saveAs: true,
          });
        } catch (e) {
          // Fermer la fenêtre « Enregistrer sous » n'est pas une erreur
          if (!/cancel/i.test(errText(e))) {
            setStatus(`Échec du téléchargement : ${errText(e)}`, true);
          }
        }
      };

      const btnOpen = document.createElement("button");
      btnOpen.type = "button";
      btnOpen.className = "action-btn";
      btnOpen.textContent = "Ouvrir";
      btnOpen.onclick = async () => {
        try {
          await browser.tabs.create({ url: mediaData.src });
        } catch (e) {
          setStatus(`Impossible d'ouvrir le lien : ${errText(e)}`, true);
        }
      };

      const btnCopy = document.createElement("button");
      btnCopy.type = "button";
      btnCopy.className = "action-btn";
      btnCopy.textContent = "Copier le lien";
      btnCopy.onclick = async () => {
        try {
          await navigator.clipboard.writeText(mediaData.src);
          btnCopy.textContent = "✓ Copié !";
          setStatus("Lien copié dans le presse-papiers.", false);
          setTimeout(() => (btnCopy.textContent = "Copier le lien"), 1500);
        } catch (e) {
          setStatus(`Copie impossible : ${errText(e)}`, true);
        }
      };

      overlay.append(btnDl, btnOpen, btnCopy);
      container.append(mediaEl, badge, overlay);
      gallery.appendChild(container);
    });
  }

  function applyFiltersAndRender() {
    const minSize = parseInt(minSizeInput.value, 10) || 0;
    try {
      localStorage.setItem(MIN_SIZE_KEY, String(minSize));
    } catch (e) {
      /* stockage indisponible : on ignore */
    }

    let medias = allMedias.filter(
      (m) => Math.max(m.width, m.height) >= minSize,
    );
    if (sortSelect.value === "size-desc") {
      medias = [...medias].sort((a, b) => b.width * b.height - a.width * a.height);
    } else if (sortSelect.value === "size-asc") {
      medias = [...medias].sort((a, b) => a.width * a.height - b.width * b.height);
    }
    currentMedias = medias;

    const filtered = allMedias.length - medias.length;
    titleText.textContent = `${medias.length} image${medias.length > 1 ? "s" : ""} (${filtered} filtrée${filtered > 1 ? "s" : ""})`;
    renderGallery(medias);
    updateDownloadAll();
  }

  // Affiche l'état de la file « Tout télécharger » portée par background.js.
  function showJob(state) {
    if (!state) return;
    if (state.running) {
      busy = true;
      btnDownloadAll.textContent = `Téléchargement ${state.done}/${state.total}`;
      setStatus(
        `Téléchargement ${state.done}/${state.total}… (vous pouvez fermer cette fenêtre)`,
        false,
      );
      updateDownloadAll();
      return;
    }
    busy = false;
    updateDownloadAll();
    const failed = state.failures.length;
    if (failed === 0) {
      setStatus(`${state.ok} fichier(s) téléchargé(s).`, false);
    } else {
      setStatus(
        `${state.ok}/${state.total} téléchargé(s), ${failed} échec(s) : ${state.failures.slice(0, 3).join(" ; ")}${failed > 3 ? " …" : ""}`,
        true,
      );
    }
  }

  browser.runtime.onMessage.addListener((msg) => {
    if (msg && msg.type === "progress") showJob(msg.state);
  });

  btnDownloadAll.onclick = async () => {
    if (busy || currentMedias.length === 0) return;
    const medias = currentMedias.map((m) => ({ src: m.src, type: m.type }));
    if (
      !confirm(
        `Télécharger les ${medias.length} fichiers ? (Le dossier par défaut sera utilisé)`,
      )
    ) {
      return;
    }
    busy = true;
    updateDownloadAll();
    try {
      const reply = await browser.runtime.sendMessage({
        type: "downloadAll",
        medias,
        names: uniqueNames(medias),
      });
      if (!reply || !reply.ok) {
        throw new Error(reply && reply.error ? reply.error : "pas de réponse");
      }
      showJob({ running: true, total: medias.length, done: 0, ok: 0, failures: [] });
    } catch (e) {
      busy = false;
      updateDownloadAll();
      setStatus(`Impossible de lancer le téléchargement : ${errText(e)}`, true);
    }
  };

  // Popup rouvert : retrouve la file en cours (ou son bilan)
  browser.runtime
    .sendMessage({ type: "getState" })
    .then(showJob)
    .catch(() => {});

  async function collect() {
    // `busy` (file de téléchargement en arrière-plan) et son statut sont conservés.
    if (!busy) setStatus("", false);
    setNotice("");
    toolbar.hidden = true;
    btnDownloadAll.hidden = true;
    titleText.textContent = "Recherche...";
    gallery.setAttribute("aria-busy", "true");
    showMessage("Analyse de la page en cours...");
    btnRefresh.disabled = true;

    let tab;
    try {
      const tabs = await browser.tabs.query({ active: true, currentWindow: true });
      tab = tabs[0];
      if (!tab) throw new Error("aucun onglet actif");
      if (tab.url && RESTRICTED_RE.test(tab.url)) {
        throw Object.assign(new Error("restreinte"), { restricted: true });
      }

      let results;
      try {
        // allFrames : couvre aussi les iframes. activeTab donne l'accès
        // temporaire à l'onglet ; si Firefox refuse un cadre, on retombe sur
        // le cadre principal seul.
        results = await browser.scripting.executeScript({
          target: { tabId: tab.id, allFrames: true },
          files: ["content.js"],
        });
      } catch (e) {
        results = await browser.scripting.executeScript({
          target: { tabId: tab.id },
          files: ["content.js"],
        });
      }

      const seen = new Set();
      allMedias = [];
      let partial = false;
      let unexportable = 0;
      for (const r of results || []) {
        const res = (r && r.result) || {};
        partial = partial || !!res.partial;
        unexportable += res.unexportable || 0;
        for (const m of res.medias || []) {
          if (!seen.has(m.src)) {
            seen.add(m.src);
            allMedias.push(m);
          }
        }
      }

      const notes = [];
      if (partial) {
        notes.push(
          "Résultats partiels : la page est trop volumineuse pour être analysée en entier. Faites défiler jusqu'aux médias voulus, puis Actualiser.",
        );
      }
      if (unexportable > 0) {
        notes.push(` canvas non exportable(s) (protégés ou trop volumineux).`);
      }
      setNotice(notes.join(" "));

      if (allMedias.length === 0) {
        titleText.textContent = "Aucune image visible";
        gallery.removeAttribute("aria-busy");
        showMessage(
          "Aucune image visible. Faites défiler la page jusqu'à une image, puis cliquez sur Actualiser.",
        );
        return;
      }
      toolbar.hidden = false;
      applyFiltersAndRender();
    } catch (error) {
      titleText.textContent = "Erreur";
      gallery.removeAttribute("aria-busy");
      if (error && error.restricted) {
        showMessage(
          "Cette page est protégée par Firefox (about:, addons.mozilla.org, etc.) : l'analyse y est impossible.",
        );
      } else {
        showMessage(`Impossible d'analyser cette page : ${errText(error)}`);
      }
      setStatus("", false);
    } finally {
      btnRefresh.disabled = false;
    }
  }

  try {
    const saved = parseInt(localStorage.getItem(MIN_SIZE_KEY), 10);
    if (saved > 0) minSizeInput.value = String(saved);
  } catch (e) {
    /* ignoré */
  }
  sortSelect.addEventListener("change", applyFiltersAndRender);
  minSizeInput.addEventListener("input", applyFiltersAndRender);
  btnRefresh.addEventListener("click", collect);
  collect();
});
