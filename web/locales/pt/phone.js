// phone — pt strings ("Usar no celular" em Configurações, components/phone.js). See docs/I18N.md.
export default {
  title: 'Usar no celular',
  intro: 'Jogue no celular ou tablet enquanto o GrandMentor roda neste computador. Os dois precisam estar no mesmo Wi-Fi.',
  loading: 'Verificando o acesso pelo celular…',
  unavailable: 'Não foi possível verificar o acesso pelo celular. Recarregue a página para tentar de novo.',
  toggle: {
    title: 'Acesso pelo celular',
    desc: 'Permite que celulares e tablets do seu Wi-Fi abram o GrandMentor com um endereço seguro (https).',
    aria: 'Permitir acesso pelo celular',
    env: 'Isso é definido quando o GrandMentor inicia (GM_LAN). Altere lá.',
  },
  restart: {
    on: 'Quase lá! Reinicie o GrandMentor para ativar o acesso pelo celular.',
    off: 'Reinicie o GrandMentor para desativar o acesso pelo celular.',
  },
  state: {
    on: 'Ativado',
    off: 'Desativado',
    plain: 'Ativado, sem https',
  },
  noNetwork: 'Não encontramos este computador em nenhuma rede. Conecte-o ao Wi-Fi e recarregue esta página.',
  plainNote: 'Outros dispositivos conseguem abrir o GrandMentor, mas sem https o celular não pode instalá-lo como aplicativo. Ative o acesso pelo celular para ter o endereço seguro.',
  pickAddress: 'Endereço',
  qrAlt: 'Código QR de {url}',
  copy: 'Copiar',
  copied: 'Endereço copiado',
  step1: {
    title: 'Instale o certificado de segurança (só uma vez)',
    text: 'Assim o celular confia no endereço seguro do GrandMentor. Escaneie o código com a câmera do celular, ou abra o link, para baixá-lo.',
    download: 'Baixar certificado',
    fingerprint: 'Impressão digital do certificado (SHA-256): {value}',
    androidTitle: 'No Android',
    android: [
      'Abra o arquivo baixado. Se nada acontecer, abra as Configurações.',
      'Vá em Segurança (ou Segurança e privacidade) → Mais configurações de segurança → Criptografia e credenciais.',
      'Toque em Instalar um certificado → Certificado CA e depois em Instalar assim mesmo.',
      'Escolha “grandmentor-ca.crt” nos seus downloads.',
    ],
    iosTitle: 'No iPhone ou iPad',
    ios: [
      'Abra o link no Safari e toque em Permitir para baixar o perfil.',
      'Vá em Ajustes → Geral → VPN e Gerenciamento de Dispositivo, toque em GrandMentor e em Instalar.',
      'Vá em Ajustes → Geral → Sobre → Ajustes de Confiança de Certificados e ative o GrandMentor.',
    ],
  },
  step2: {
    title: 'Abra o GrandMentor no celular',
    text: 'Escaneie este código ou digite o endereço no Chrome (Safari no iPhone).',
    install: 'Para deixá-lo na tela inicial, abra o menu do navegador e escolha Instalar app (ou Adicionar à tela inicial).',
  },
  step3: {
    title: 'Digite o PIN',
    text: 'Na primeira vez, o celular vai pedir este PIN. Depois ele continua conectado.',
    pinAria: 'PIN de acesso: {pin}',
    newPin: 'Novo PIN',
    newPinDone: 'Novo PIN pronto. Os dispositivos já conectados continuam conectados.',
  },
  devices: {
    title: 'Dispositivos conectados',
    count: {
      zero: 'Nenhum celular ou tablet está conectado.',
      one: '{count} dispositivo está conectado.',
      other: '{count} dispositivos estão conectados.',
    },
    signOutAll: 'Desconectar todos',
    confirmTitle: 'Desconectar todos os dispositivos?',
    confirmText: 'Todos os celulares e tablets vão precisar do PIN de novo. Este computador não é afetado.',
    done: 'Todos os dispositivos foram desconectados.',
  },
  off: {
    text: 'Quando estiver ativado, você verá aqui um código para escanear, um PIN e passos simples.',
  },
  remote: {
    text: 'Você está usando o GrandMentor em outro dispositivo. As configurações do celular ficam no computador que o executa.',
    signOut: 'Desconectar este dispositivo',
  },
  safety: 'Só dispositivos do seu Wi-Fi que sabem o PIN podem abrir o GrandMentor. Este computador nunca precisa do PIN.',
};
