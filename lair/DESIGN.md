# The Lair's design

The Lair is the Dragon's ledger and the Scales' market. Each view answers one question and offers one move. This
file is the visual system the pages are built from. The code in `styles.css` is the source of truth for every value.

## World

The night inside the Dragon's hide: a deep violet ground, a second violet layer for forms and dialogs, and gold kept
for value and the primary action. The Dragon art (`art.js`) and its three moods carry the brand, with the WYRM
wordmark in blackletter.

## Colour

| Token | Value | Job |
|---|---|---|
| `--ground` | `#14101d` | Page. The Dragon page tints it by mood: Sleeping `#11121d`, Breathing `#1a1018`, Hunting `#10132a`. |
| `--layer`, `--layer-2` | `#1c1628`, `#261e35` | Forms, dialogs, buttons. |
| `--line`, `--line-2` | `#2d2540`, `#3d3354` | Row rules, gridlines, control edges. |
| `--text`, `--muted`, `--faint` | `#eee8f6`, `#a69cba`, `#6d6382` | Text, secondary text, axis labels. |
| `--gold` | `#f4b942` | Primary buttons, prices, the Dragon's buy coins, the current nav and tab bar, focus rings. Nothing else. |

Scale status is the main colour signal and the same everywhere (grid, rows, Scale page, legend):

| Status | Fill | Also |
|---|---|---|
| Yours | lilac `#b49cff` | |
| For sale | gold | Any listed Scale, yours included, gets a gold edge. |
| Held | slate `#6f6489` | |
| Creator's | verdigris `#5bb39b` | Scales 91 to 100 while their creator holds them unlisted. |
| Unsold | none, dashed edge | Not bought from the sale yet. |
| Retired | `#1d1628`, faint number | Unsold when the sale closed. |

Hold tiers (Ash, Ember, Molten, Obsidian) are words only, never colour.

## Type

One family, Bricolage Grotesque, self-hosted in `fonts/` as three static latin subsets (400 and 600 at text optical
size, 800 at display size), with tabular figures everywhere. The wordmark uses a four-letter Grenze Gotisch subset.
Scale: body 1rem, small .875rem, h2 1.375rem, h1 2.5rem (2rem on phones), figures 1.875rem (2.125rem in the hoard,
1.5rem on phones), the mood word up to 6rem. Headings track -0.02em, the mood word -0.04em.

## Components

- **Plate.** `scaleSvg(status, label, listed)`: one faceted shield per Scale, filled by status.
- **The hide.** The 100 plates in ten offset rows that overlap like scales. It is the signature of the site. Filter
  chips name each status with its count and dim the rest.
- **Tabs.** Underlined links with `role=tab`, each its own address (`#/scales/sale|market|all|yours`). Arrow keys,
  Home and End move between them.
- **Rows.** Lists of Scales, listings and offers: identity, one figure, actions, ruled lines and no cards.
- **Figures.** A small label over a large tabular number with its unit, on the ground with no box.
- **Price chart.** `chart.js`: hand-drawn SVG at the pixel width shown, a line with a soft lilac fill, gridlines only
  at labelled prices, time labels from the samples, a gold coin at each logged buy, a crosshair with a readout and
  span chips. It never draws a threshold, step, reference or forecast line.
- **Buttons.** 10 px radius. Gold primary, violet default, quiet text-only. One primary per view, so a button repeated
  on every row of a list (Buy, Claim, Accept) is never gold.

## Space and motion

Content width 1120 px. Sections are 64 px apart, and the gaps inside them are 12 to 32 px. Motion only answers an
action (150 to 250 ms ease-out). The exceptions are the Dragon's own mood animation, the 400 ms ground tint when
its mood changes, and the toast entrance. Reduced
motion turns all of it off.
