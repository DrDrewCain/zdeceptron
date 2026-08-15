# The spacetrader wars

This file is on disk and `zdc build` reads it, exactly as it reads the
posts in `content/`. What is new is the fence below: it names a component,
and the built page shows a live one.

```zd RingChart
slug: spacetrader-wars
```

The chart above is an ordinary `component`. It holds `client` state, it
responds to clicks, and it would be written the same way if the page had
named it rather than this file. What the fence buys is that a *file* may
ask for one — and only for one the program declared.

An ordinary fence is still an ordinary code block, and nothing about it
changed:

```js
const shipsLost = 41;
```

A second widget, to show that two parts of a document may each be one, and
that the prose between them is its own node:

```zd StackBars
slug: spacetrader-wars
series: losses
```

Everything after the last fence is one more run of prose. A `<script>` in
this file would still be *shown* rather than run, because each run goes
through the same renderer `build markdown` does.
