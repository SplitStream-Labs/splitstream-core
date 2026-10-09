#!/usr/bin/env python3
"""Assemble the unified SplitStream documentation site.

The three SplitStream repositories each own a ``docs/`` directory and are
synced to their own GitBook spaces. This script copies all three into a single
MkDocs ``docs_dir`` so one GitHub Pages site can serve them together, and
rewrites the links that used to point at the separate GitBook sites into
relative links inside the combined site.

Layout produced (paths are relative to the ``--out`` directory)::

    index.md                    # docs-hub/ landing page
    architecture.md             # docs-hub/ cross-repo architecture page
    assets/                     # banner and other static assets
    core/        *.md           # splitstream-core docs
    actions/     *.md           # splitstream-actions docs
    sdk-cli/     *.md           # splitstream-sdk-cli docs

Usage::

    python scripts/assemble_docs.py \
        --core docs \
        --actions ../splitstream-actions/docs \
        --sdk-cli ../splitstream-sdk-cli/docs \
        --hub docs-hub \
        --assets assets \
        --out .docs-build/docs
"""

from __future__ import annotations

import argparse
import re
import shutil
from pathlib import Path

# Repo key -> GitBook slug. The keys are also the on-site directory names.
SLUG_TO_KEY = {
    "splitstream-core": "core",
    "splitstream-actions": "actions",
    "splitstream-sdk-cli": "sdk-cli",
}

# https://splitstream.gitbook.io/splitstream-actions/for-maintainers
# -> <prefix>/actions/for-maintainers.md   (or introduction.md when bare)
GITBOOK_LINK = re.compile(
    r"https://splitstream\.gitbook\.io/"
    r"(splitstream-core|splitstream-actions|splitstream-sdk-cli)"
    r"(?:/([A-Za-z0-9/_-]+))?/?"
)

# The three repos were originally published under a personal account; the
# canonical organisation is SplitStream-Labs.
LEGACY_ACCOUNT = re.compile(r"https://github\.com/Oyinkans0la12(?=[/\s)\"']|$)")

# Repository-root files (../CONTRIBUTING.md, ../SECURITY.md, ...) are not part
# of the rendered site. Point them at the owning repository on GitHub instead.
ROOT_DOC_LINK = re.compile(r"\.\./+(CONTRIBUTING|SECURITY|README)\.md")

# Files that exist only to drive the GitBook table of contents.
SKIP_FILES = {"SUMMARY.md"}


def rewrite_links(
    text: str,
    prefix: str,
    rewrite_gitbook: bool,
    repo_slug: str | None = None,
) -> str:
    """Rewrite cross-site links in one markdown document.

    ``prefix`` is inserted in front of generated relative paths (``"../"`` for
    documents one directory deep, ``""`` for the hub pages at the site root).
    ``repo_slug`` is the GitHub repository the document belongs to, used to
    resolve links to repository-root files that are not part of the site.
    """
    if repo_slug:
        text = ROOT_DOC_LINK.sub(
            rf"https://github.com/SplitStream-Labs/{repo_slug}/blob/main/\1.md",
            text,
        )

    if rewrite_gitbook:

        def _sub(match: re.Match[str]) -> str:
            key = SLUG_TO_KEY[match.group(1)]
            page = (match.group(2) or "introduction").strip("/")
            return f"{prefix}{key}/{page}.md"

        text = GITBOOK_LINK.sub(_sub, text)

    text = LEGACY_ACCOUNT.sub("https://github.com/SplitStream-Labs", text)
    return text


def copy_markdown_tree(
    src: Path,
    dest: Path,
    prefix: str,
    rewrite_gitbook: bool,
    repo_slug: str | None = None,
) -> int:
    """Copy every markdown file from ``src`` into ``dest``, rewriting links."""
    if not src.is_dir():
        raise SystemExit(f"error: docs directory not found: {src}")

    dest.mkdir(parents=True, exist_ok=True)
    count = 0
    for path in sorted(src.rglob("*")):
        if path.is_dir():
            continue
        rel = path.relative_to(src)
        if path.name in SKIP_FILES:
            continue
        target = dest / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        if path.suffix == ".md":
            target.write_text(
                rewrite_links(
                    path.read_text(encoding="utf-8"),
                    prefix,
                    rewrite_gitbook,
                    repo_slug=repo_slug,
                ),
                encoding="utf-8",
            )
        else:
            shutil.copy2(path, target)
        count += 1
    return count


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--core", required=True, help="splitstream-core docs directory")
    parser.add_argument("--actions", required=True, help="splitstream-actions docs directory")
    parser.add_argument("--sdk-cli", required=True, help="splitstream-sdk-cli docs directory")
    parser.add_argument("--hub", required=True, help="docs-hub directory (index/architecture)")
    parser.add_argument("--assets", required=True, help="static assets directory")
    parser.add_argument("--out", required=True, help="output docs_dir for MkDocs")
    args = parser.parse_args()

    out = Path(args.out).resolve()
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    sections = {
        "core": Path(args.core).resolve(),
        "actions": Path(args.actions).resolve(),
        "sdk-cli": Path(args.sdk_cli).resolve(),
    }

    copied: dict[str, int] = {}
    for key, src in sections.items():
        copied[key] = copy_markdown_tree(
            src,
            out / key,
            prefix="../",
            rewrite_gitbook=True,
            repo_slug=f"splitstream-{key}",
        )

    hub = Path(args.hub).resolve()
    copied["hub"] = copy_markdown_tree(
        hub, out, prefix="", rewrite_gitbook=True
    )

    assets = Path(args.assets).resolve()
    if assets.is_dir():
        shutil.copytree(assets, out / "assets", dirs_exist_ok=True)

    print("Assembled docs into", out)
    for key, total in copied.items():
        print(f"  {key:<8} {total} file(s)")


if __name__ == "__main__":
    main()
