# 009 — Hover feedback on nested tool disclosures

Priority: visual. Status: open.

Tool expanders and nested Input/Output/Error headers/submenus lack their own hover
recognition; only the root Details section lights up. Give every interactive
header/menu item a visible, subdued hover/press/focus state using its actual clipped
hit region. Parent and child highlights must not compound into a strong wash.

Check collapsed/expanded tools, nested large sections, submenu transitions, pointer
edges, reflow while streaming, context menus and touch. Coordinate colour/ripple
with 005/008. Reuse existing hit geometry, not a second hover tree.
