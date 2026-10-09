// practice — de strings (opening practice vs a bot). See docs/I18N.md.
export default {
  yourLine: "deine Variante",
  setup: {
    title: "Training: {name}",
    detail: "Die Eröffnungszüge stehen schon auf dem Brett. Die Partie zählt wie jede andere.",
  },
  game: {
    banner: "Training: {name}",
    leftBookUser: "Hier hast du die Theorie verlassen. Die Hauptvariante war {san}. Kein Problem, spiel weiter!",
    leftBookUserPlain: "Hier hast du die Theorievariante verlassen. Kein Problem, spiel weiter!",
    leftBookBot: "{name} hat die Theorievariante verlassen. Vertrau jetzt deinen eigenen Ideen!",
    outOfBook: "Die bekannten Theoriezüge sind vorbei. Ab hier zählen deine eigenen Ideen!",
  },
  notes: "Eröffnungstraining: {name}",
  error: {
    notFound: "Diese Eröffnung haben wir nicht gefunden, hier ist eine normale Partie.",
    invalid: "Diese Eröffnungsvariante hat nicht funktioniert, hier ist eine normale Partie.",
    tooLong: "Diese Variante ist zu lang (höchstens {count} Züge), hier ist eine normale Partie.",
    finished: "Diese Variante beendet die Partie schon, hier ist eine normale Partie.",
  },
  cta: {
    practice: "Gegen einen Bot üben",
    practiceLine: "Diese Variante gegen einen Bot üben",
    afterTrain: "Jetzt gegen einen Bot spielen",
    hint: "Spiele eine echte Partie, die mit diesen Zügen beginnt",
  },
};
