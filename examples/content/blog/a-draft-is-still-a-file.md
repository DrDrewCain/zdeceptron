# A draft is still a file

This post is marked as a draft, so the view never shows it. It is still
read at build time, and it is still on disk beside the others.

The distinction matters: `keep each post where not post.draft` is a
build-time filter over a build-time list, so the draft's rendered HTML
never reaches the bundle. Nothing is hidden from the reader by the
browser, because nothing was sent to the browser to hide.
