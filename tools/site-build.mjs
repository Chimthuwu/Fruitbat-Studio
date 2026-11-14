// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

// Cloudflare Pages runs `npm run build` (see the Pages project settings). The landing page is already static in
// `site/`, so there is nothing to compile: this copies it to `dist/` and leaves `site/` in place, so the Pages
// project works whether its output directory is set to `site` or to `dist`.

import { cpSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const from = join(root, "site");
const to = join(root, "dist");

rmSync(to, { recursive: true, force: true });
mkdirSync(to, { recursive: true });
cpSync(from, to, { recursive: true });

console.log(`site -> dist (${from} -> ${to})`);
