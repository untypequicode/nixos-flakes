// Fonction pour deviner et forcer la bonne extension de fichier
function getSmartFilename(url, type) {
  const defaultExt = type === "video" ? "mp4" : "jpg";

  // Cas 1 : Les images codées en texte direct (Base64)
  if (url.startsWith("data:")) {
    const mime = url.split(";")[0].split(":")[1];
    const ext = mime ? mime.split("/")[1] : defaultExt;
    return `media_${Date.now()}.${ext}`;
  }

  try {
    // Cas 2 : Une URL classique
    const urlObj = new URL(url);
    let filename = urlObj.pathname.split("/").pop();

    // S'il manque l'extension dans le nom (ex: "photo_profil" au lieu de "photo.jpg")
    if (!filename.includes(".")) {
      // On cherche si le site a mis le format dans l'url (ex: ?format=png)
      const format = urlObj.searchParams.get("format") || defaultExt;
      filename = filename
        ? `${filename}.${format}`
        : `media_${Date.now()}.${format}`;
    }
    return filename;
  } catch (e) {
    return `media_${Date.now()}.${defaultExt}`;
  }
}

document.addEventListener("DOMContentLoaded", async () => {
  const gallery = document.getElementById("gallery");
  const titleText = document.getElementById("title-text");
  const btnDownloadAll = document.getElementById("download-all");
  const toolbar = document.getElementById("toolbar");
  const sortSelect = document.getElementById("sort-select");
  const minSizeInput = document.getElementById("min-size");

  let allMedias = [];

  // Construit une galerie à partir d'une liste de médias déjà filtrée/triée
  function renderGallery(medias) {
    gallery.innerHTML = "";

    if (medias.length === 0) {
      gallery.innerHTML =
        "<div class='message'>Aucun média ne correspond à ce filtre.</div>";
      return;
    }

    medias.forEach((mediaData) => {
      const container = document.createElement("div");
      container.className = "img-container";

      // Création de l'élément Image OU Vidéo
      const mediaEl = document.createElement(
        mediaData.type === "video" ? "video" : "img",
      );
      mediaEl.src = mediaData.src;
      if (mediaData.type === "video") {
        mediaEl.muted = true; // Pour ne pas faire de bruit inattendu
        mediaEl.autoplay = true; // Joue un aperçu de la vidéo
        mediaEl.loop = true;
      }

      // Badge de résolution
      const badge = document.createElement("div");
      badge.className = "resolution-badge";
      badge.textContent = `${mediaData.width} × ${mediaData.height}`;

      // Badge spécial Vidéo
      if (mediaData.type === "video") {
        const typeBadge = document.createElement("div");
        typeBadge.className = "type-badge";
        typeBadge.textContent = "🎬 VIDÉO";
        container.appendChild(typeBadge);
      }

      // Menu superposé
      const overlay = document.createElement("div");
      overlay.className = "overlay";

      const btnOpen = document.createElement("button");
      btnOpen.className = "action-btn";
      btnOpen.textContent = "Ouvrir";
      btnOpen.onclick = () => browser.tabs.create({ url: mediaData.src });

      const btnDl = document.createElement("button");
      btnDl.className = "action-btn primary";
      btnDl.textContent = "Télécharger (Choix)";
      btnDl.onclick = () => {
        let filename = getSmartFilename(mediaData.src, mediaData.type);
        browser.downloads.download({
          url: mediaData.src,
          filename: filename,
          saveAs: true,
        });
      };

      const btnCopy = document.createElement("button");
      btnCopy.className = "action-btn";
      btnCopy.textContent = "Copier le lien";
      btnCopy.onclick = () => {
        navigator.clipboard.writeText(mediaData.src);
        btnCopy.textContent = "✓ Copié !";
        setTimeout(() => (btnCopy.textContent = "Copier le lien"), 1500);
      };

      overlay.appendChild(btnDl);
      overlay.appendChild(btnOpen);
      overlay.appendChild(btnCopy);

      container.appendChild(mediaEl);
      if (mediaData.width > 0) container.appendChild(badge);
      container.appendChild(overlay);

      gallery.appendChild(container);
    });
  }

  // Applique le filtre de taille minimale + le tri choisis, puis redessine
  function applyFiltersAndRender() {
    const minSize = parseInt(minSizeInput.value, 10) || 0;

    let medias = allMedias.filter(
      (m) => Math.max(m.width, m.height) >= minSize,
    );

    if (sortSelect.value === "size-desc") {
      medias = [...medias].sort(
        (a, b) => b.width * b.height - a.width * a.height,
      );
    } else if (sortSelect.value === "size-asc") {
      medias = [...medias].sort(
        (a, b) => a.width * a.height - b.width * b.height,
      );
    }

    titleText.textContent = `${medias.length} / ${allMedias.length} Élément${allMedias.length > 1 ? "s" : ""}`;
    renderGallery(medias);

    // Le bouton "tout télécharger" respecte lui aussi le filtre actif
    btnDownloadAll.onclick = () => {
      if (
        confirm(
          `Télécharger les ${medias.length} fichiers ? (Le dossier par défaut sera utilisé)`,
        )
      ) {
        medias.forEach((mediaData) => {
          let filename = getSmartFilename(mediaData.src, mediaData.type);
          browser.downloads.download({
            url: mediaData.src,
            filename: filename,
            saveAs: false,
          });
        });
      }
    };
  }

  try {
    let tabs = await browser.tabs.query({ active: true, currentWindow: true });
    let activeTab = tabs[0];

    // Injection à la demande : content.js n'est plus déclaré dans le manifest,
    // il n'est exécuté que sur l'onglet actif, au moment où le popup s'ouvre.
    const [{ result }] = await browser.scripting.executeScript({
      target: { tabId: activeTab.id },
      files: ["content.js"],
    });

    allMedias = result || [];

    if (allMedias.length > 0) {
      btnDownloadAll.style.display = "block";
      toolbar.style.display = "flex";

      sortSelect.addEventListener("change", applyFiltersAndRender);
      minSizeInput.addEventListener("input", applyFiltersAndRender);

      applyFiltersAndRender();
    } else {
      titleText.textContent = "Aucun média";
      gallery.innerHTML =
        "<div class='message'>Aucune image ou vidéo visible actuellement.</div>";
    }
  } catch (error) {
    titleText.textContent = "Erreur";
    gallery.innerHTML =
      "<div class='message'>Impossible d'analyser cette page.</div>";
  }
});
