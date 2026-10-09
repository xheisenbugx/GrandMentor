// phone — en strings ("Use on your phone" in Settings, components/phone.js). See docs/I18N.md.
export default {
  title: 'Use on your phone',
  intro: 'Play on your phone or tablet while GrandMentor runs on this computer. Both need to be on the same Wi-Fi.',
  loading: 'Checking phone access…',
  unavailable: 'Couldn’t check phone access. Reload the page to try again.',
  toggle: {
    title: 'Phone access',
    desc: 'Lets phones and tablets on your Wi-Fi open GrandMentor with a secure (https) address.',
    aria: 'Allow phone access',
    env: 'This is set when GrandMentor starts (GM_LAN). Change it there.',
  },
  restart: {
    on: 'Almost there! Restart GrandMentor to turn phone access on.',
    off: 'Restart GrandMentor to turn phone access off.',
  },
  state: {
    on: 'On',
    off: 'Off',
    plain: 'On, without https',
  },
  noNetwork: 'We couldn’t find this computer on a network. Connect it to Wi-Fi, then reload this page.',
  plainNote: 'Other devices can reach GrandMentor, but without https your phone can’t install it as an app. Turn on phone access for the secure address.',
  pickAddress: 'Address',
  qrAlt: 'QR code for {url}',
  copy: 'Copy',
  copied: 'Address copied',
  step1: {
    title: 'Install the security certificate (once)',
    text: 'This lets your phone trust GrandMentor’s secure address. Scan the code with your phone’s camera, or open the link, to download it.',
    download: 'Download certificate',
    fingerprint: 'Certificate fingerprint (SHA-256): {value}',
    androidTitle: 'On Android',
    android: [
      'Open the downloaded file. If nothing happens, open Settings instead.',
      'Go to Security (or Security & privacy) → More security settings → Encryption & credentials.',
      'Tap Install a certificate → CA certificate, then tap Install anyway.',
      'Choose “grandmentor-ca.crt” from your downloads.',
    ],
    iosTitle: 'On iPhone or iPad',
    ios: [
      'Open the link in Safari and tap Allow to download the profile.',
      'Go to Settings → General → VPN & Device Management, tap GrandMentor and Install.',
      'Go to Settings → General → About → Certificate Trust Settings and switch on GrandMentor.',
    ],
  },
  step2: {
    title: 'Open GrandMentor on your phone',
    text: 'Scan this code or type the address in Chrome (Safari on iPhone).',
    install: 'To keep it on your home screen, open the browser menu and choose Install app (or Add to Home screen).',
  },
  step3: {
    title: 'Enter the PIN',
    text: 'The first time, your phone asks for this PIN. It stays signed in after that.',
    pinAria: 'Access PIN: {pin}',
    newPin: 'New PIN',
    newPinDone: 'New PIN ready. Devices already signed in stay signed in.',
  },
  devices: {
    title: 'Signed-in devices',
    count: {
      zero: 'No phones or tablets are signed in.',
      one: '{count} device is signed in.',
      other: '{count} devices are signed in.',
    },
    signOutAll: 'Sign out all devices',
    confirmTitle: 'Sign out all devices?',
    confirmText: 'Every phone and tablet will need the PIN again. This computer is not affected.',
    done: 'All devices are signed out.',
  },
  off: {
    text: 'When it’s on, you’ll see a code to scan, a PIN and easy steps here.',
  },
  remote: {
    text: 'You’re using GrandMentor from another device. Phone settings live on the computer that runs it.',
    signOut: 'Sign out this device',
  },
  safety: 'Only devices on your Wi-Fi that know the PIN can open GrandMentor. This computer never needs the PIN.',
};
