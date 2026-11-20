// Copyright 2026 Zyvor AI Labs · https://zyvor.dev
// SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

import { AuthShell } from "@/components/AuthShell";
import { ForgotForm } from "@/components/AuthForms";

export default function Forgot() {
  return <AuthShell title="Reset your password" lede="Enter your email and we will send you a link to choose a new one."><ForgotForm /></AuthShell>;
}
