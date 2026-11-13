// Page d'événements (MV3) : porte la file « Tout télécharger » pour qu'elle
// survive à la fermeture du popup. Le popup envoie la liste par
// runtime.sendMessage et reçoit la progression ; s'il est rouvert, il
// redemande l'état en cours (message « getState »).
const CONCURRENCY = 3;

// État de la file en cours (ou du dernier bilan non encore lu). L'état vit en
// mémoire : si Firefox décharge la page pendant une longue file, le bilan est perdu.
let job = null;
// id de téléchargement -> fonction appelée à la fin (complete / interrupted)
const waiting = new Map();

const errText = (e) => (e && e.message ? e.message : String(e));

function publish() {
  // Aucun popup ouvert : sendMessage rejette, ce n'est pas une erreur.
  browser.runtime
    .sendMessage({ type: "progress", state: job })
    .catch(() => {});
}

browser.downloads.onChanged.addListener((delta) => {
  const done = waiting.get(delta.id);
  if (!done || !delta.state) return;
  if (delta.state.current === "complete") {
    done(null);
  } else if (delta.state.current === "interrupted") {
    done(delta.error && delta.error.current ? delta.error.current : "interrompu");
  }
});

// Attend la fin réelle d'un téléchargement ; renvoie null si réussi, sinon la raison.
async function untilDone(id) {
  const finished = new Promise((resolve) => waiting.set(id, resolve));
  try {
    // Peut déjà être terminé avant l'enregistrement de l'écouteur
    const [item] = await browser.downloads.search({ id });
    if (item && item.state === "complete") waiting.get(id)(null);
    else if (item && item.state === "interrupted") {
      waiting.get(id)(item.error || "interrompu");
    }
  } catch (e) {
    /* on attend l'événement */
  }
  const reason = await finished;
  waiting.delete(id);
  return reason;
}

async function runJob(medias, names) {
  let next = 0;
  const worker = async () => {
    while (next < medias.length) {
      const i = next++;
      try {
        const id = await browser.downloads.download({
          url: medias[i].src,
          filename: names[i],
          saveAs: false,
          conflictAction: "uniquify",
        });
        const reason = await untilDone(id);
        if (reason) job.failures.push(`${names[i]} (${reason})`);
        else job.ok += 1;
      } catch (e) {
        job.failures.push(`${names[i]} (${errText(e)})`);
      }
      job.done += 1;
      publish();
    }
  };
  await Promise.all(Array.from({ length: CONCURRENCY }, worker));
  job.running = false;
  publish();
}

browser.runtime.onMessage.addListener((msg) => {
  if (!msg || typeof msg.type !== "string") return undefined;

  if (msg.type === "getState") {
    const state = job;
    // Un bilan terminé n'est remis qu'une fois
    if (job && !job.running) job = null;
    return Promise.resolve(state);
  }

  if (msg.type === "downloadAll") {
    if (job && job.running) {
      return Promise.resolve({ ok: false, error: "un téléchargement est déjà en cours" });
    }
    const medias = Array.isArray(msg.medias) ? msg.medias : [];
    const names = Array.isArray(msg.names) ? msg.names : [];
    if (medias.length === 0 || names.length !== medias.length) {
      return Promise.resolve({ ok: false, error: "liste vide" });
    }
    job = { running: true, total: medias.length, done: 0, ok: 0, failures: [] };
    runJob(medias, names);
    return Promise.resolve({ ok: true });
  }
  return undefined;
});
