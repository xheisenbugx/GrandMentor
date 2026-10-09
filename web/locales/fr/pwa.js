// pwa — fr strings. See docs/I18N.md.
export default {
  install: {
    button: 'Installer l’appli',
    title: 'Installer GrandMentor',
    installed: 'GrandMentor est installé. Tu le trouveras avec tes autres applis.',
    iosTitle: 'Ajoute GrandMentor à ton écran d’accueil',
    iosIntro: 'Installe GrandMentor pour l’ouvrir comme une appli, en plein écran, même sans Internet.',
    iosStep1: 'Touche le bouton **Partager** dans la barre d’outils de Safari.',
    iosStep2: 'Fais défiler et choisis **Sur l’écran d’accueil**.',
    iosStep3: 'Touche **Ajouter**. GrandMentor apparaîtra à côté de tes autres applis.',
    gotIt: 'Compris',
  },
  offline: {
    backOnline: 'De nouveau en ligne',
    retry: 'Réessayer',
    retryLabel: 'Vérifier à nouveau la connexion',
  },
  update: {
    title: 'Une nouvelle version est disponible',
    text: 'Recharge pour profiter des dernières améliorations.',
    reload: 'Recharger',
  },
  queue: {
    saved: 'Enregistré sur cet appareil. On le synchronisera dès ton retour en ligne.',
    synced: {
      one: '{count} résultat obtenu hors ligne synchronisé.',
      many: '{count} résultats obtenus hors ligne synchronisés.',
      other: '{count} résultats obtenus hors ligne synchronisés.',
    },
  },
  connection: {
    server: {
      title: "Le moteur de GrandMentor n’est pas lancé",
      text: "Les problèmes et les leçons déjà ouverts marchent toujours ; jouer contre les bots, analyser et revoir une partie ont besoin du moteur. Lance-le sur ton ordinateur et cette page se reconnectera toute seule.",
    },
    device: {
      title: "Tu es hors ligne",
      text: "Cet appareil n’a pas de connexion et ne joint pas GrandMentor. Les problèmes et les leçons déjà ouverts marchent toujours.",
    },
  },
};
