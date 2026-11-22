# Keel website

The marketing landing page for Keel. Plain HTML, CSS and a little JavaScript, no build step.

- `index.html` is the page, `styles.css` the styles, `main.js` the tabs, copy buttons and scroll effects.
- Links to `docs/` and `docs/tutorial/` point at the mdBook documentation, which is built into `site/docs/` from the top-level `docs/` folder.

Preview locally:

```sh
python3 -m http.server -d site 8000   # then open http://localhost:8000
```
