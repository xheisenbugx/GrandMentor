// practice — es strings (opening practice vs a bot). See docs/I18N.md.
export default {
  yourLine: "tu línea",
  setup: {
    title: "Practicando: {name}",
    detail: "Las jugadas de la apertura ya están en el tablero. La partida cuenta como cualquier otra.",
  },
  game: {
    banner: "Practicando: {name}",
    leftBookUser: "Aquí saliste del libro. La línea principal era {san}. ¡Tranquilo, sigue jugando!",
    leftBookUserPlain: "Aquí saliste de la línea del libro. ¡Tranquilo, sigue jugando!",
    leftBookBot: "{name} salió de la línea del libro. ¡Ahora confía en tus ideas!",
    outOfBook: "Ya pasaste las jugadas del libro. ¡Desde aquí, usa tus propias ideas!",
  },
  notes: "Práctica de apertura: {name}",
  error: {
    notFound: "No encontramos esa apertura, así que aquí tienes una partida normal.",
    invalid: "Esa línea de apertura no funcionó, así que aquí tienes una partida normal.",
    tooLong: "Esa línea es demasiado larga (máximo {count} jugadas), así que aquí tienes una partida normal.",
    finished: "Esa línea ya termina la partida, así que aquí tienes una partida normal.",
  },
  cta: {
    practice: "Practicar contra un bot",
    practiceLine: "Practicar esta línea contra un bot",
    afterTrain: "Ahora juégala contra un bot",
    hint: "Juega una partida real que empieza con estas jugadas",
  },
};
