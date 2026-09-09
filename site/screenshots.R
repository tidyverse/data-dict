#!/usr/bin/env Rscript

# Capture light and dark screenshots of the rendered otters dictionary for the
# landing page, saved to site/images/otters-{light,dark}.png. Run from the repo
# root:
#
#   Rscript site/screenshots.R
#
# Requires Google Chrome, the chromote package, and the CLI built
# (`cargo build -p data-dict-cli --release`) or `DATA_DICT` pointing at a
# binary. The otters repo is cloned because render-spec profiles the source
# data into the page.

library(chromote)

bin <- Sys.getenv("DATA_DICT", "target/release/data-dict")

src <- tempfile("otters-")
status <- system2(
  "git",
  c("clone", "--quiet", "--depth", "1", "https://github.com/hadley/otters.git", src)
)
if (status != 0) {
  stop("failed to clone hadley/otters", call. = FALSE)
}

# Render the copy in site/examples: that is the one the site shows.
dict <- file.path(src, "data-dict.yaml")
file.copy(file.path("site", "examples", "otters.yaml"), dict, overwrite = TRUE)
html <- file.path(src, "otters.html")
if (system2(bin, c("render-spec", dict, "--output", html)) != 0) {
  stop("render-spec failed", call. = FALSE)
}

out_dir <- file.path("site", "images")
dir.create(out_dir, showWarnings = FALSE)

shoot <- function(scheme) {
  b <- ChromoteSession$new(width = 1440, height = 900)
  on.exit(b$close())
  b$Emulation$setEmulatedMedia(features = list(
    list(name = "prefers-color-scheme", value = scheme)
  ))
  b$Page$navigate(paste0("file://", normalizePath(html)))
  b$screenshot(file.path(out_dir, paste0("otters-", scheme, ".png")))
}

shoot("light")
shoot("dark")
