// Ce script n'est plus déclaré dans le manifest : il est injecté à la demande
// par popup.js via browser.scripting.executeScript, uniquement sur l'onglet actif,
// et seulement quand l'utilisateur ouvre le popup. Cela évite de faire tourner du
// code sur CHAQUE page visitée (gain de perf + moins de surface d'exposition).
(() => {
  const visibleMedia = [];
  const seenUrls = new Set();

  // Fonction pour ajouter un média trouvé (et nettoyer l'URL)
  const addMedia = (src, type, width, height) => {
    if (!src || src === "none" || src.startsWith("blob:")) return;

    // Nettoyage des URL issues du CSS (ex: url("mon-image.jpg"))
    if (src.startsWith("url(")) {
      src = src.slice(4, -1).replace(/["']/g, "");
    }

    // Transformer les URL relatives en URL absolues utilisables
    if (!src.startsWith("http") && !src.startsWith("data:")) {
      try {
        src = new URL(src, window.location.href).href;
      } catch (e) {
        return;
      }
    }

    if (!seenUrls.has(src)) {
      seenUrls.add(src);
      visibleMedia.push({
        src,
        type,
        width: Math.round(width),
        height: Math.round(height),
      });
    }
  };

  // Fonction pour explorer TOUT le DOM du site, même les zones cachées (Shadow DOM)
  const getAllNodes = (root = document) => {
    const nodes = [];
    const traverse = (node) => {
      if (node.nodeType === Node.ELEMENT_NODE) {
        nodes.push(node);
        if (node.shadowRoot) traverse(node.shadowRoot); // Pénètre les boucliers Shadow DOM
      }
      node.childNodes.forEach(traverse);
    };
    traverse(root);
    return nodes;
  };

  const allElements = getAllNodes(document.body);

  // Analyse de CHAQUE élément de la page
  allElements.forEach((el) => {
    // 1. L'élément est-il dans la zone visible de l'écran ?
    const rect = el.getBoundingClientRect();
    const isVisible =
      rect.width > 0 &&
      rect.height > 0 &&
      rect.bottom > 0 &&
      rect.right > 0 &&
      rect.top <
        (window.innerHeight || document.documentElement.clientHeight) &&
      rect.left < (window.innerWidth || document.documentElement.clientWidth);

    if (!isVisible) return;

    const tagName = el.tagName.toLowerCase();
    // On récupère la taille réelle ou la taille affichée
    const w = el.naturalWidth || el.videoWidth || el.width || rect.width || 0;
    const h =
      el.naturalHeight || el.videoHeight || el.height || rect.height || 0;

    // TECHNIQUE 1 : Balises <img> classiques (contourne les balises <picture> et les images responsives)
    if (tagName === "img") {
      // currentSrc permet d'avoir la vraie image affichée, pas celle par défaut
      addMedia(el.currentSrc || el.src, "img", w, h);
    }

    // TECHNIQUE 2 : Vidéos et leurs miniatures (poster)
    else if (tagName === "video") {
      let vSrc =
        el.currentSrc ||
        el.src ||
        (el.querySelector("source") ? el.querySelector("source").src : null);
      addMedia(vSrc, "video", w, h);
      if (el.poster) addMedia(el.poster, "img", w, h); // Récupère la miniature de la vidéo
    }

    // TECHNIQUE 3 : Balises <canvas> (Images dessinées en JavaScript)
    else if (tagName === "canvas") {
      try {
        // On capture le dessin du canvas pour en faire une image PNG
        addMedia(el.toDataURL("image/png"), "img", w, h);
      } catch (e) {
        // Ignoré si bloqué par une sécurité CORS du serveur
      }
    }

    // TECHNIQUE 4 : Images de fond CSS (La méthode n°1 des sites "anti-téléchargement")
    const style = window.getComputedStyle(el);
    if (style.backgroundImage && style.backgroundImage !== "none") {
      // Une balise peut avoir plusieurs images de fond (on les récupère toutes)
      const bgUrls = style.backgroundImage.match(/url\([^)]+\)/g);
      if (bgUrls) {
        bgUrls.forEach((bgUrl) =>
          addMedia(bgUrl, "img", rect.width, rect.height),
        );
      }
    }
  });

  // Valeur de complétion du script : récupérée par
  // results[0].result côté popup.js
  return visibleMedia;
})();
