// phone — es strings ("Usar en tu móvil" en Ajustes, components/phone.js). See docs/I18N.md.
export default {
  title: 'Usar en tu móvil',
  intro: 'Juega en tu móvil o tableta mientras GrandMentor se ejecuta en este ordenador. Los dos tienen que estar en la misma wifi.',
  loading: 'Comprobando el acceso desde el móvil…',
  unavailable: 'No pudimos comprobar el acceso desde el móvil. Recarga la página para intentarlo de nuevo.',
  toggle: {
    title: 'Acceso desde el móvil',
    desc: 'Permite que los móviles y tabletas de tu wifi abran GrandMentor con una dirección segura (https).',
    aria: 'Permitir el acceso desde el móvil',
    env: 'Esto se fija al iniciar GrandMentor (GM_LAN). Cámbialo ahí.',
  },
  restart: {
    on: '¡Casi listo! Reinicia GrandMentor para activar el acceso desde el móvil.',
    off: 'Reinicia GrandMentor para desactivar el acceso desde el móvil.',
  },
  state: {
    on: 'Activado',
    off: 'Desactivado',
    plain: 'Activado, sin https',
  },
  noNetwork: 'No encontramos este ordenador en ninguna red. Conéctalo a la wifi y recarga esta página.',
  plainNote: 'Otros dispositivos pueden abrir GrandMentor, pero sin https tu móvil no puede instalarlo como aplicación. Activa el acceso desde el móvil para tener la dirección segura.',
  pickAddress: 'Dirección',
  qrAlt: 'Código QR de {url}',
  copy: 'Copiar',
  copied: 'Dirección copiada',
  step1: {
    title: 'Instala el certificado de seguridad (solo una vez)',
    text: 'Así tu móvil confía en la dirección segura de GrandMentor. Escanea el código con la cámara del móvil, o abre el enlace, para descargarlo.',
    download: 'Descargar certificado',
    fingerprint: 'Huella del certificado (SHA-256): {value}',
    androidTitle: 'En Android',
    android: [
      'Abre el archivo descargado. Si no pasa nada, abre Ajustes.',
      'Ve a Seguridad (o Seguridad y privacidad) → Más ajustes de seguridad → Cifrado y credenciales.',
      'Toca Instalar un certificado → Certificado de CA y después Instalar de todos modos.',
      'Elige «grandmentor-ca.crt» en tus descargas.',
    ],
    iosTitle: 'En iPhone o iPad',
    ios: [
      'Abre el enlace en Safari y toca Permitir para descargar el perfil.',
      'Ve a Ajustes → General → VPN y gestión de dispositivos, toca GrandMentor e Instalar.',
      'Ve a Ajustes → General → Información → Ajustes de confianza de certificados y activa GrandMentor.',
    ],
  },
  step2: {
    title: 'Abre GrandMentor en tu móvil',
    text: 'Escanea este código o escribe la dirección en Chrome (Safari en iPhone).',
    install: 'Para tenerlo en la pantalla de inicio, abre el menú del navegador y elige Instalar aplicación (o Añadir a pantalla de inicio).',
  },
  step3: {
    title: 'Escribe el PIN',
    text: 'La primera vez, tu móvil te pedirá este PIN. Después seguirá conectado.',
    pinAria: 'PIN de acceso: {pin}',
    newPin: 'Nuevo PIN',
    newPinDone: 'Nuevo PIN listo. Los dispositivos ya conectados siguen conectados.',
  },
  devices: {
    title: 'Dispositivos conectados',
    count: {
      zero: 'No hay ningún móvil ni tableta conectados.',
      one: 'Hay {count} dispositivo conectado.',
      other: 'Hay {count} dispositivos conectados.',
    },
    signOutAll: 'Cerrar sesión en todos',
    confirmTitle: '¿Cerrar sesión en todos los dispositivos?',
    confirmText: 'Todos los móviles y tabletas tendrán que volver a escribir el PIN. Este ordenador no se ve afectado.',
    done: 'Se cerró la sesión en todos los dispositivos.',
  },
  off: {
    text: 'Cuando lo actives, aquí verás un código para escanear, un PIN y unos pasos sencillos.',
  },
  remote: {
    text: 'Estás usando GrandMentor desde otro dispositivo. Los ajustes del móvil están en el ordenador donde se ejecuta.',
    signOut: 'Cerrar sesión en este dispositivo',
  },
  safety: 'Solo los dispositivos de tu wifi que conozcan el PIN pueden abrir GrandMentor. Este ordenador nunca necesita el PIN.',
};
