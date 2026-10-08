// review — es strings. See docs/I18N.md.
export default {
  title: 'Revisión de la partida',

  // Frases completas: {move} es la jugada (SAN, a veces con su número).
  phrase: {
    brilliant: '{move} es brillante',
    great: '{move} es una gran jugada',
    best: '{move} es la mejor jugada',
    excellent: '{move} es excelente',
    good: '{move} es buena',
    book: '{move} es una jugada de teoría',
    inaccuracy: '{move} es una imprecisión',
    mistake: '{move} es un error',
    miss: '{move} es una oportunidad perdida',
    blunder: '{move} es un error grave',
    forced: '{move} es forzada',
    played: 'se jugó {move}',
    works: '{move} funciona',
    notBest: '{move} no es la mejor',
  },

  loading: {
    game: 'Cargando la partida…',
    title: 'Analizando tu partida…',
    subtitle: '{white} vs {black} · {moves}',
    moves: { one: '{count} jugada', other: '{count} jugadas' },
    cancel: 'Cancelar',
    done: '¡Listo! Preparando tu informe…',
    steps: [
      'Leyendo la apertura…',
      'Revisando cada jugada con el motor…',
      'Buscando tácticas que se te escaparon…',
      'Encontrando tus mejores jugadas…',
      'Buscando ideas brillantes…',
      'Escribiendo el informe de tu entrenador…',
    ],
    tips: [
      'La precisión mide cuánto se acercaron tus jugadas a las mejores jugadas del motor.',
      'Un error grave (??) es una jugada que tira por la borda gran parte de tu ventaja.',
      'Las jugadas brillantes (!!) suelen ser buenos sacrificios de pieza, difíciles de encontrar.',
      'Los momentos clave son los puntos de inflexión de la partida: repítelos para aprender más.',
      'Las jugadas de teoría son jugadas de apertura conocidas que usan los jugadores fuertes.',
      'Una «oportunidad perdida» significa que pasaste por alto la ocasión de ganar material o la partida.',
    ],
  },

  error: {
    badLinkTitle: 'No encontramos esa partida',
    badLinkText: 'Parece que el enlace está roto. Elige una partida de tu biblioteca para revisarla.',
    notFoundTitle: 'Partida no encontrada',
    notFoundText: 'Puede que se haya borrado.',
    loadTitle: 'No se pudo cargar la partida',
    emptyTitle: 'Todavía no hay nada que revisar',
    emptyText: 'Esta partida no tiene jugadas. ¡Haz algunas jugadas y vuelve para que tu entrenador la revise!',
    playBot: 'Jugar contra un bot',
    goLibrary: 'Ir a la biblioteca',
    reviewEmpty: 'La revisión llegó vacía.',
    failedTitle: 'La revisión no pudo terminar',
    generic: 'Algo salió mal.',
    tryAgain: 'Reintentar',
    openAnalysis: 'Abrir en análisis',
  },

  side: {
    white: 'Blancas',
    black: 'Negras',
  },

  tabs: {
    review: 'Revisión',
    coach: 'Pregunta al entrenador',
  },

  nav: {
    first: 'Principio (Inicio)',
    prev: 'Anterior (←)',
    next: 'Siguiente (→)',
    last: 'Final (Fin)',
    flip: 'Girar tablero',
    analyse: 'Abrir en el tablero de análisis',
    analyseTip: 'Analizar esta posición',
  },

  mentor: {
    greeting: '¿Tienes curiosidad por alguna jugada? Ve a ella en el tablero y pregúntame; por ejemplo: *«¿Por qué fue un error?»*',
    suggestions: [
      '¿Por qué fue mala esta jugada?',
      '¿Cuál era la idea de la mejor jugada?',
      '¿Qué debería aprender de esta partida?',
      '¿Cuál es el plan aquí?',
    ],
  },

  report: {
    accuracy: 'Precisión',
    gameRating: 'Puntuación de la partida',
    vs: 'vs',
    classifications: 'Clasificación de las jugadas',
    keyMoments: 'Momentos clave',
    defaultSummary: 'Jugaste con un **{accuracy}% de precisión**. Repasemos juntos la partida y encontremos los momentos clave.',
  },

  footer: {
    start: 'Empezar revisión',
    backToGame: 'Volver a la partida',
    backToReport: 'Volver al informe',
    prev: 'Anterior',
    next: 'Siguiente',
  },

  walk: {
    introTitle: '¡Vamos a revisar tu partida!',
    intro: 'Pulsa **Siguiente** (o la tecla →) para avanzar jugada a jugada. Te señalaré las mejores jugadas, los errores y lo que podrías haber jugado en su lugar.\n\n¿Quieres probar una idea? **Mueve cualquier pieza en el tablero** para probar tu propia línea; la comprobaré con el motor.',
    opening: 'Apertura: **{name}**',
    showLine: 'Ver línea',
    retry: 'Reintentar',
    nextKey: 'Siguiente momento clave',
    keyMoment: 'Momento clave',
    bestWas: 'Lo mejor era {move}',
    evalBefore: '({eval} antes de tu jugada)',
    tryTip: 'Mueve una pieza en el tablero para probar tu propia línea desde aquí.',
    bestLine: 'Mejor línea',
    stop: 'Detener',
  },

  retry: {
    promptWhite: 'Busca una jugada mejor para las **blancas**. En la partida, {played}.',
    promptBlack: 'Busca una jugada mejor para las **negras**. En la partida, {played}.',
    titleSuccess: '¡Bien hecho!',
    titleError: 'No exactamente',
    titleTurn: 'Tu turno: inténtalo de nuevo',
    hint: 'Pista',
    showAnswer: 'Ver solución',
    backToGame: 'Volver a la partida',
    continue: 'Continuar',
    hintText: 'Fíjate en la pieza resaltada: tiene una jugada fuerte.',
    answer: 'La mejor jugada era **{move}**. Línea: {line}',
    bestFirstTry: '¡**{move}** es la mejor jugada! A la primera: impresionante.',
    bestLater: '¡**{move}** es la mejor jugada! Lo conseguiste.',
    sameAsGame: 'Esa es la jugada que hiciste en la partida ({move}). ¡Busca algo mejor!',
    alsoGood: '¡{verdict} también! {explanation}',
    notGood: '{verdict}. {explanation}',
    tryAgain: '¡Inténtalo de nuevo!',
    checkFailed: 'Ahora mismo no he podido comprobar esa jugada. Prueba la mejor jugada o pulsa *Ver solución*.',
  },

  explore: {
    thinking: 'El motor está pensando…',
    depth: '· profundidad {depth}',
    checking: 'Comprobando tu jugada…',
    title: 'Tu propia línea',
    notInGame: 'No está en la partida',
    fromStart: 'Línea alternativa desde la posición inicial. Sigue moviendo piezas para profundizar, o usa ← para retroceder.',
    fromMove: 'Línea alternativa tras {move}. Sigue moviendo piezas para profundizar, o usa ← para retroceder.',
    undo: 'Deshacer jugada',
    backToGame: 'Volver a la partida',
  },
};
