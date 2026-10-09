// practice — pt strings (opening practice vs a bot). See docs/I18N.md.
export default {
  yourLine: "sua linha",
  setup: {
    title: "Praticando: {name}",
    detail: "Os lances da abertura já estão no tabuleiro. A partida conta como qualquer outra.",
  },
  game: {
    banner: "Praticando: {name}",
    leftBookUser: "Aqui você saiu do livro. A linha principal era {san}. Tudo bem, continue jogando!",
    leftBookUserPlain: "Aqui você saiu da linha do livro. Tudo bem, continue jogando!",
    leftBookBot: "{name} saiu da linha do livro. Agora confie nas suas ideias!",
    outOfBook: "Você já passou dos lances do livro. Daqui em diante, use suas próprias ideias!",
  },
  notes: "Prática de abertura: {name}",
  error: {
    notFound: "Não encontramos essa abertura, então aqui está uma partida normal.",
    invalid: "Essa linha de abertura não funcionou, então aqui está uma partida normal.",
    tooLong: "Essa linha é longa demais (no máximo {count} lances), então aqui está uma partida normal.",
    finished: "Essa linha já termina a partida, então aqui está uma partida normal.",
  },
  cta: {
    practice: "Praticar contra um bot",
    practiceLine: "Praticar esta linha contra um bot",
    afterTrain: "Agora jogue contra um bot",
    hint: "Jogue uma partida real que começa com estes lances",
  },
};
