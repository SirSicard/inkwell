/**
 * The hero demo's dictations, invented. `live` is what the Drop shows while the key is held: the
 * live model's provisional words, unpunctuated. `final` is what the final pass types when the key
 * comes up (README: "the text that is typed comes from the final pass when you let go").
 */
export const DEMO_LINES = [
  { live: 'can you send me the notes from tuesday before lunch', final: 'Can you send me the notes from Tuesday before lunch?' },
  { live: 'the draft looks good ship it once the tests pass', final: 'The draft looks good. Ship it once the tests pass.' },
  { live: 'remind me to book the room for thursday at ten', final: 'Remind me to book the room for Thursday at 10.' },
] as const;
