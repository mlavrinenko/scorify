// The same score drawn as SVG (top) and as native items (bottom); the two
// halves should be indistinguishable.
#import "../lib.typ": score
#set page(width: 140mm, height: auto, margin: 5mm)

#let args = (
  staves: (
    (clef: "treble", music: "c'8n[1] d'8 e'8 f#'8 color{#ff0000:g'4} <c' e' g'>4 | a'2~ a'4 r4 | b'1"),
    (clef: "bass", music: "c2 g,2 | f,4 e,8 d,8 c,2 | g,,1"),
  ),
  key: "G",
  time: "4/4",
  staff-group: "grand",
)

#score(..args)
#line(length: 100%, stroke: 0.2pt + gray)
#score(..args, output: "native")
