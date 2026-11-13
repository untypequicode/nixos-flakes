// Injecté à la demande par popup.js (scripting.executeScript, allFrames) sur
// l'onglet actif uniquement. Aucun état global : réinjecter le script est sans
// effet de bord, chaque exécution renvoie simplement son propre résultat :
// { medias, partial, unexportable }.
(() => {
  const MAX_RESULTS = 300; // plafond de médias renvoyés
  const TIME_BUDGET_MS = 150; // budget du parcours des images de fond CSS
  const MIN_CANVAS_PX = 32; // les canvas plus petits sont ignorés (pixels de suivi, icônes)
  const MAX_CANVAS_PIXELS = 16 * 1024 * 1024; // au-delà, on n'essaie même pas d'exporter
  const MAX_DATA_URL_LENGTH = 6 * 1024 * 1024; // plafond du data: URL d'un canvas
  const visibleMedia = [];
  const seenUrls = new Set();
  let partial = false; // le parcours a été interrompu : résultats incomplets
  let unexportable = 0; // canvas tainted, trop gros ou à data: URL trop long

  const addMedia = (src, type, width, height) => {
    if (visibleMedia.length >= MAX_RESULTS) return;
    if (!src || src === "none" || src.startsWith("blob:")) return;

    // URL issues du CSS (ex: url("mon-image.jpg"))
    if (src.startsWith("url(")) {
      src = src.slice(4, -1).replace(/["']/g, "");
    }

    if (!/^(https?:|data:)/i.test(src)) {
      try {
        src = new URL(src, window.location.href).href;
      } catch (e) {
        return;
      }
      if (!/^https?:/i.test(src)) return;
    }

    if (!seenUrls.has(src)) {
      seenUrls.add(src);
      visibleMedia.push({
        src,
        type,
        width: Math.round(width) || 0,
        height: Math.round(height) || 0,
      });
    }
  };

  const vw = window.innerWidth || document.documentElement.clientWidth;
  const vh = window.innerHeight || document.documentElement.clientHeight;

  const inViewport = (rect) =>
    rect.width > 0 &&
    rect.height > 0 &&
    rect.bottom > 0 &&
    rect.right > 0 &&
    rect.top < vh &&
    rect.left < vw;

  // Masqué par display/visibility/opacity ? (appelé seulement après le filtre
  // géométrique, car getComputedStyle est le point coûteux.)
  const isHidden = (el, style) => {
    if (style.display === "none" || style.visibility === "hidden") return true;
    if (parseFloat(style.opacity) === 0) return true;
    if (typeof el.checkVisibility === "function") {
      try {
        return !el.checkVisibility({
          opacityProperty: true,
          visibilityProperty: true,
        });
      } catch (e) {
        /* option non gérée : on garde le résultat ci-dessus */
      }
    }
    return false;
  };

  // Élément affiché dans la fenêtre : rect visible, puis style calculé.
  const visibleRect = (el) => {
    const rect = el.getBoundingClientRect();
    if (!inViewport(rect)) return null;
    if (isHidden(el, window.getComputedStyle(el))) return null;
    return rect;
  };

  const addBackgrounds = (style, rect) => {
    const bg = style.backgroundImage;
    if (!bg || bg === "none") return;
    const bgUrls = bg.match(/url\((?:"[^"]*"|'[^']*'|[^)]*)\)/g);
    if (bgUrls) {
      bgUrls.forEach((u) => addMedia(u, "img", rect.width, rect.height));
    }
  };

  // --- 1. Collecte ciblée : images, vidéos, canvas, <picture>, SVG ---------

  for (const el of document.images) {
    if (visibleMedia.length >= MAX_RESULTS) break;
    const rect = visibleRect(el);
    if (!rect) continue;
    addMedia(
      el.currentSrc || el.src,
      "img",
      el.naturalWidth || el.width || rect.width,
      el.naturalHeight || el.height || rect.height,
    );
  }

  let targets = [];
  try {
    targets = document.querySelectorAll(
      "video, canvas, picture source, svg image",
    );
  } catch (e) {
    /* sélecteur non géré : on continue sans */
  }

  for (const el of targets) {
    if (visibleMedia.length >= MAX_RESULTS) break;
    const tag = el.tagName.toLowerCase();

    if (tag === "source") {
      // <source> n'a pas de boîte : on regarde le <picture>. Si l'<img> du
      // <picture> est présent, il a déjà donné l'image réellement affichée.
      const picture = el.parentElement;
      if (!picture || picture.querySelector("img")) continue;
      const rect = visibleRect(picture);
      if (!rect) continue;
      const first = (el.srcset || "").split(",")[0].trim().split(/\s+/)[0];
      addMedia(first, "img", rect.width, rect.height);
      continue;
    }

    const rect = visibleRect(el);
    if (!rect) continue;

    if (tag === "video") {
      const source = el.querySelector("source");
      const w = el.videoWidth || el.width || rect.width;
      const h = el.videoHeight || el.height || rect.height;
      addMedia(
        el.currentSrc || el.src || (source ? source.src : null),
        "video",
        w,
        h,
      );
      if (el.poster) addMedia(el.poster, "img", w, h);
    } else if (tag === "canvas") {
      if (el.width < MIN_CANVAS_PX || el.height < MIN_CANVAS_PX) continue;
      if (el.width * el.height > MAX_CANVAS_PIXELS) {
        unexportable += 1;
        continue;
      }
      try {
        const url = el.toDataURL("image/png");
        if (url.length > MAX_DATA_URL_LENGTH) {
          unexportable += 1;
        } else {
          addMedia(url, "img", el.width, el.height);
        }
      } catch (e) {
        // Canvas « tainted » (contenu cross-origin) : non exportable
        unexportable += 1;
      }
    } else if (tag === "image") {
      // <image> SVG : href (ou xlink:href), éventuellement relatif
      const href =
        el.getAttribute("href") ||
        el.getAttributeNS("http://www.w3.org/1999/xlink", "href");
      addMedia(href, "img", rect.width, rect.height);
    }
  }

  // --- 2. Images de fond CSS : parcours qui saute les sous-arbres inutiles --

  const deadline = performance.now() + TIME_BUDGET_MS;
  const roots = [document.body || document.documentElement];
  const seenRoots = new Set();

  // Les sous-arbres sans affichage (display:none) ou entièrement hors de la
  // fenêtre sont rejetés d'un coup (le contenu qui déborde de sa boîte est un
  // compromis assumé, d'où le caractère « partiel » possible des résultats).
  const walkerFilter = (node) => {
    const rect = node.getBoundingClientRect();
    if (rect.width > 0 || rect.height > 0) {
      const outside =
        rect.bottom <= 0 ||
        rect.right <= 0 ||
        rect.top >= vh ||
        rect.left >= vw;
      return outside ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
    }
    // Boîte vide : display:none (rien dessous) ou display:contents / flottant
    // sans taille (les enfants peuvent s'afficher)
    return window.getComputedStyle(node).display === "none"
      ? NodeFilter.FILTER_REJECT
      : NodeFilter.FILTER_SKIP;
  };

  scan: while (roots.length) {
    const root = roots.shift();
    if (!root || seenRoots.has(root)) continue;
    seenRoots.add(root);
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {
      acceptNode: walkerFilter,
    });
    for (let el = walker.nextNode(); el; el = walker.nextNode()) {
      if (visibleMedia.length >= MAX_RESULTS) break scan;
      if (performance.now() > deadline) {
        partial = true;
        break scan;
      }
      if (el.shadowRoot) roots.push(el.shadowRoot);
      const style = window.getComputedStyle(el);
      if (isHidden(el, style)) continue;
      addBackgrounds(style, el.getBoundingClientRect());
    }
  }
  // Racines Shadow DOM restantes si le parcours a été interrompu
  if (roots.some((r) => r && !seenRoots.has(r))) partial = true;
  if (visibleMedia.length >= MAX_RESULTS) partial = true;

  return { medias: visibleMedia, partial, unexportable };
})();
