# scry

A very simple command line tool for extracting text from
[Scrivener](https://www.literatureandlatte.com/scrivener/overview)
projects to enable processing by downstream command line utilities.

Scrivener is awesome at many things but automation ain't one of them.
This at least makes it easy to surface context text or notes for
further processing so, with some ingenuity, you can embed your own
markup, todos, etc.

It's intended to be simple and fast and to do one thing well. It's not
a replacement project compile (for which see more ambitious projects
elsewhere on github...).

```
scry -h
```

By default `scry`` extracts all paragraphs from the _draft_ folder of the
project, stripping RTF controls and other artefacts of Scrivener
styling and annotation.

Various command line flags are available to select other meaningful
bits of text. For example:

Inline annotations and notes from the draft folder:

```
scry proj.scrivx -in
```

Content from the research folder:

```
scry proj.scrivx -r
```

## Acknowledgement

Currently, much of the RTF processing is lifted more or less directly from
https://github.com/compenguy/rtf2text which is `Copyright 2019 Will
Page <compenguy@gmail.com>`. `
