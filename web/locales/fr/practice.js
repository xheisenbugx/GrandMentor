// practice — fr strings (opening practice vs a bot). See docs/I18N.md.
export default {
  yourLine: "ta ligne",
  setup: {
    title: "Entraînement : {name}",
    detail: "Les coups de l'ouverture sont déjà joués. La partie compte comme les autres.",
  },
  game: {
    banner: "Entraînement : {name}",
    leftBookUser: "Tu as quitté la théorie ici. La ligne principale était {san}. Pas de souci, continue !",
    leftBookUserPlain: "Tu as quitté la ligne théorique ici. Pas de souci, continue !",
    leftBookBot: "{name} a quitté la ligne théorique. Fais confiance à tes idées !",
    outOfBook: "Tu as dépassé les coups connus. À partir d'ici, place à tes propres idées !",
  },
  notes: "Entraînement d'ouverture : {name}",
  error: {
    notFound: "Nous n'avons pas trouvé cette ouverture, voici donc une partie normale.",
    invalid: "Cette ligne d'ouverture n'a pas fonctionné, voici donc une partie normale.",
    tooLong: "Cette ligne est trop longue ({count} coups maximum), voici donc une partie normale.",
    finished: "Cette ligne termine déjà la partie, voici donc une partie normale.",
  },
  cta: {
    practice: "S'entraîner contre un bot",
    practiceLine: "Jouer cette ligne contre un bot",
    afterTrain: "Maintenant, joue-la contre un bot",
    hint: "Joue une vraie partie qui commence par ces coups",
  },
};
