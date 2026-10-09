// practice — en strings (opening practice vs a bot). See docs/I18N.md.
export default {
  yourLine: "your line",
  setup: {
    title: "Practising: {name}",
    detail: "The opening moves are already on the board. The game counts like any other.",
  },
  game: {
    banner: "Practising: {name}",
    leftBookUser: "You left the book here. The main line was {san}. No worries, keep playing!",
    leftBookUserPlain: "You left the book line here. No worries, keep playing!",
    leftBookBot: "{name} left the book line. Now trust your own ideas!",
    outOfBook: "The known opening moves are over. From here, it's your own ideas!",
  },
  notes: "Opening practice: {name}",
  error: {
    notFound: "We couldn't find that opening, so here's a normal game setup.",
    invalid: "That opening line didn't work, so here's a normal game setup.",
    tooLong: "That line is too long (at most {count} moves), so here's a normal game setup.",
    finished: "That line already ends the game, so here's a normal game setup.",
  },
  cta: {
    practice: "Practice vs a bot",
    practiceLine: "Practice this line vs a bot",
    afterTrain: "Now play it against a bot",
    hint: "Play a real game that starts with these moves",
  },
};
