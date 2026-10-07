# Documentation site

Use Hugo 0.167.0 or later. From the repository root:

```sh
hugo server --source site
```

Open the URL printed by Hugo. To build static HTML:

```sh
hugo --source site --minify
```

Edit the existing Markdown in `docs/`; Hugo mounts it directly. Roadmap files
are excluded. The landing page lives in `site/content/_index.md`; templates
and plain CSS live in `site/layouts/` and `site/assets/`. Hugo minifies and
fingerprints the stylesheet when building the site.

Relative Markdown links resolve to site pages when published, or GitHub source
files otherwise. HTML rendering is enabled for the guides' `<details>` blocks.

For hosting, choose **Settings → Pages → Source → GitHub Actions** in the
repository. The Pages workflow builds pull requests and deploys pushes to
`master`. Generated output and local tools are ignored by Git.
