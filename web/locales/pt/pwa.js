// pwa — pt strings. See docs/I18N.md.
export default {
  install: {
    button: 'Instalar app',
    title: 'Instale o GrandMentor',
    installed: 'O GrandMentor está instalado. Ele fica junto com seus outros apps.',
    iosTitle: 'Adicione o GrandMentor à Tela de Início',
    iosIntro: 'Instale o GrandMentor para abri-lo como um app, em tela cheia, mesmo sem internet.',
    iosStep1: 'Toque no botão **Compartilhar** na barra do Safari.',
    iosStep2: 'Role para baixo e escolha **Adicionar à Tela de Início**.',
    iosStep3: 'Toque em **Adicionar**. O GrandMentor aparecerá junto com seus outros apps.',
    gotIt: 'Entendi',
  },
  offline: {
    backOnline: 'Conectado de novo',
    retry: 'Tentar de novo',
    retryLabel: 'Verificar a conexão de novo',
  },
  update: {
    title: 'Há uma nova versão disponível',
    text: 'Recarregue para ter as últimas melhorias.',
    reload: 'Recarregar',
  },
  queue: {
    saved: 'Salvo neste dispositivo. Vamos sincronizar quando você estiver online de novo.',
    synced: { zero: '{count} resultados feitos offline foram sincronizados.', one: '{count} resultado feito offline foi sincronizado.', other: '{count} resultados feitos offline foram sincronizados.' },
  },
  connection: {
    server: {
      title: "O motor do GrandMentor não está rodando",
      text: "Os problemas e as lições que você já abriu continuam funcionando; jogar contra bots, analisar e revisar partidas precisam do motor. Inicie-o no seu computador e esta página se reconecta sozinha.",
    },
    device: {
      title: "Você está sem conexão",
      text: "Este dispositivo está sem conexão e não alcança o GrandMentor. Os problemas e as lições que você já abriu continuam funcionando.",
    },
  },
};
