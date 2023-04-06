//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vitejs.dev/config/
export default defineConfig({
  base: '',
  plugins: [react()],
})
