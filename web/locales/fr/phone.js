// phone — fr strings (« Utiliser sur ton téléphone » dans Réglages, components/phone.js). See docs/I18N.md.
export default {
  title: 'Utiliser sur ton téléphone',
  intro: 'Joue sur ton téléphone ou ta tablette pendant que GrandMentor tourne sur cet ordinateur. Les deux doivent être sur le même Wi-Fi.',
  loading: 'Vérification de l’accès depuis le téléphone…',
  unavailable: 'Impossible de vérifier l’accès depuis le téléphone. Recharge la page pour réessayer.',
  toggle: {
    title: 'Accès depuis le téléphone',
    desc: 'Permet aux téléphones et tablettes de ton Wi-Fi d’ouvrir GrandMentor avec une adresse sécurisée (https).',
    aria: 'Autoriser l’accès depuis le téléphone',
    env: 'Ce réglage est fixé au démarrage de GrandMentor (GM_LAN). Modifie-le là.',
  },
  restart: {
    on: 'Presque fini ! Redémarre GrandMentor pour activer l’accès depuis le téléphone.',
    off: 'Redémarre GrandMentor pour désactiver l’accès depuis le téléphone.',
  },
  state: {
    on: 'Activé',
    off: 'Désactivé',
    plain: 'Activé, sans https',
  },
  noNetwork: 'Nous n’avons trouvé cet ordinateur sur aucun réseau. Connecte-le au Wi-Fi, puis recharge cette page.',
  plainNote: 'D’autres appareils peuvent ouvrir GrandMentor, mais sans https ton téléphone ne peut pas l’installer comme une appli. Active l’accès depuis le téléphone pour avoir l’adresse sécurisée.',
  pickAddress: 'Adresse',
  qrAlt: 'QR code pour {url}',
  copy: 'Copier',
  copied: 'Adresse copiée',
  step1: {
    title: 'Installe le certificat de sécurité (une seule fois)',
    text: 'Ainsi, ton téléphone fait confiance à l’adresse sécurisée de GrandMentor. Scanne le code avec l’appareil photo, ou ouvre le lien, pour le télécharger.',
    download: 'Télécharger le certificat',
    fingerprint: 'Empreinte du certificat (SHA-256) : {value}',
    androidTitle: 'Sur Android',
    android: [
      'Ouvre le fichier téléchargé. S’il ne se passe rien, ouvre plutôt les Paramètres.',
      'Va dans Sécurité (ou Sécurité et confidentialité) → Plus de paramètres de sécurité → Chiffrement et identifiants.',
      'Touche Installer un certificat → Certificat CA, puis Installer quand même.',
      'Choisis « grandmentor-ca.crt » dans tes téléchargements.',
    ],
    iosTitle: 'Sur iPhone ou iPad',
    ios: [
      'Ouvre le lien dans Safari et touche Autoriser pour télécharger le profil.',
      'Va dans Réglages → Général → VPN et gestion de l’appareil, touche GrandMentor puis Installer.',
      'Va dans Réglages → Général → Informations → Réglages des certificats et active GrandMentor.',
    ],
  },
  step2: {
    title: 'Ouvre GrandMentor sur ton téléphone',
    text: 'Scanne ce code ou tape l’adresse dans Chrome (Safari sur iPhone).',
    install: 'Pour le garder sur l’écran d’accueil, ouvre le menu du navigateur et choisis Installer l’appli (ou Ajouter à l’écran d’accueil).',
  },
  step3: {
    title: 'Saisis le code PIN',
    text: 'La première fois, ton téléphone demande ce code PIN. Ensuite, il reste connecté.',
    pinAria: 'Code PIN d’accès : {pin}',
    newPin: 'Nouveau PIN',
    newPinDone: 'Nouveau code PIN prêt. Les appareils déjà connectés le restent.',
  },
  devices: {
    title: 'Appareils connectés',
    count: {
      zero: 'Aucun téléphone ni aucune tablette n’est connecté.',
      one: '{count} appareil est connecté.',
      other: '{count} appareils sont connectés.',
    },
    signOutAll: 'Déconnecter tous les appareils',
    confirmTitle: 'Déconnecter tous les appareils ?',
    confirmText: 'Chaque téléphone et tablette devra de nouveau saisir le code PIN. Cet ordinateur n’est pas concerné.',
    done: 'Tous les appareils sont déconnectés.',
  },
  off: {
    text: 'Une fois activé, tu verras ici un code à scanner, un code PIN et des étapes simples.',
  },
  remote: {
    text: 'Tu utilises GrandMentor depuis un autre appareil. Les réglages du téléphone se trouvent sur l’ordinateur qui l’exécute.',
    signOut: 'Déconnecter cet appareil',
  },
  safety: 'Seuls les appareils de ton Wi-Fi qui connaissent le code PIN peuvent ouvrir GrandMentor. Cet ordinateur n’en a jamais besoin.',
};
