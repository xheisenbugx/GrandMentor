// phone — de strings („Auf dem Handy nutzen“ in den Einstellungen, components/phone.js). See docs/I18N.md.
export default {
  title: 'Auf dem Handy nutzen',
  intro: 'Spiel auf deinem Handy oder Tablet, während GrandMentor auf diesem Computer läuft. Beide müssen im selben WLAN sein.',
  loading: 'Handy-Zugang wird geprüft…',
  unavailable: 'Der Handy-Zugang konnte nicht geprüft werden. Lade die Seite neu und versuch es noch einmal.',
  toggle: {
    title: 'Handy-Zugang',
    desc: 'Erlaubt Handys und Tablets in deinem WLAN, GrandMentor über eine sichere Adresse (https) zu öffnen.',
    aria: 'Handy-Zugang erlauben',
    env: 'Das wird beim Start von GrandMentor festgelegt (GM_LAN). Ändere es dort.',
  },
  restart: {
    on: 'Fast geschafft! Starte GrandMentor neu, um den Handy-Zugang einzuschalten.',
    off: 'Starte GrandMentor neu, um den Handy-Zugang auszuschalten.',
  },
  state: {
    on: 'An',
    off: 'Aus',
    plain: 'An, ohne https',
  },
  noNetwork: 'Dieser Computer ist in keinem Netzwerk zu finden. Verbinde ihn mit dem WLAN und lade die Seite neu.',
  plainNote: 'Andere Geräte erreichen GrandMentor, aber ohne https kann dein Handy es nicht als App installieren. Schalte den Handy-Zugang ein, um die sichere Adresse zu bekommen.',
  pickAddress: 'Adresse',
  qrAlt: 'QR-Code für {url}',
  copy: 'Kopieren',
  copied: 'Adresse kopiert',
  step1: {
    title: 'Sicherheitszertifikat installieren (nur einmal)',
    text: 'Damit vertraut dein Handy der sicheren Adresse von GrandMentor. Scanne den Code mit der Handykamera oder öffne den Link, um es herunterzuladen.',
    download: 'Zertifikat herunterladen',
    fingerprint: 'Fingerabdruck des Zertifikats (SHA-256): {value}',
    androidTitle: 'Auf Android',
    android: [
      'Öffne die heruntergeladene Datei. Wenn nichts passiert, öffne stattdessen die Einstellungen.',
      'Gehe zu Sicherheit (oder Sicherheit & Datenschutz) → Weitere Sicherheitseinstellungen → Verschlüsselung und Anmeldedaten.',
      'Tippe auf Zertifikat installieren → CA-Zertifikat und dann auf Trotzdem installieren.',
      'Wähle „grandmentor-ca.crt“ aus deinen Downloads.',
    ],
    iosTitle: 'Auf iPhone oder iPad',
    ios: [
      'Öffne den Link in Safari und tippe auf Erlauben, um das Profil zu laden.',
      'Gehe zu Einstellungen → Allgemein → VPN und Geräteverwaltung, tippe auf GrandMentor und Installieren.',
      'Gehe zu Einstellungen → Allgemein → Info → Zertifikatsvertrauenseinstellungen und schalte GrandMentor ein.',
    ],
  },
  step2: {
    title: 'GrandMentor auf dem Handy öffnen',
    text: 'Scanne diesen Code oder tippe die Adresse in Chrome ein (Safari auf dem iPhone).',
    install: 'Damit es auf dem Startbildschirm bleibt, öffne das Browsermenü und wähle App installieren (oder Zum Startbildschirm hinzufügen).',
  },
  step3: {
    title: 'PIN eingeben',
    text: 'Beim ersten Mal fragt dein Handy nach dieser PIN. Danach bleibt es angemeldet.',
    pinAria: 'Zugangs-PIN: {pin}',
    newPin: 'Neue PIN',
    newPinDone: 'Neue PIN bereit. Bereits angemeldete Geräte bleiben angemeldet.',
  },
  devices: {
    title: 'Angemeldete Geräte',
    count: {
      zero: 'Kein Handy und kein Tablet ist angemeldet.',
      one: '{count} Gerät ist angemeldet.',
      other: '{count} Geräte sind angemeldet.',
    },
    signOutAll: 'Alle Geräte abmelden',
    confirmTitle: 'Alle Geräte abmelden?',
    confirmText: 'Jedes Handy und Tablet braucht dann wieder die PIN. Dieser Computer ist nicht betroffen.',
    done: 'Alle Geräte sind abgemeldet.',
  },
  off: {
    text: 'Sobald er an ist, findest du hier einen Code zum Scannen, eine PIN und einfache Schritte.',
  },
  remote: {
    text: 'Du nutzt GrandMentor auf einem anderen Gerät. Die Handy-Einstellungen findest du auf dem Computer, auf dem es läuft.',
    signOut: 'Dieses Gerät abmelden',
  },
  safety: 'Nur Geräte in deinem WLAN, die die PIN kennen, können GrandMentor öffnen. Dieser Computer braucht nie eine PIN.',
};
