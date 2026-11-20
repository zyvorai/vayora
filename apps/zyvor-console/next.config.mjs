// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

export default {
  // The workspace SDK ships as ESM from dist/.
  transpilePackages: ["@zyvor/node"],
  // a document upload goes through a server action: up to 5 MB of file, a third more as base64 form data
  experimental: { serverActions: { bodySizeLimit: "7mb" } },
  agentRules: false,
};
