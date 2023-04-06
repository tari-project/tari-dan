//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import React from 'react'
import ReactDOM from 'react-dom/client'
import { createBrowserRouter, RouterProvider } from 'react-router-dom'
import App from './App'
import './index.css'
import AllowedPages from './routes/AllowedPages/AllowedPages'

const router = createBrowserRouter([
  {
    path: '*',
    element: <App />,
    // errorElement: <ErrorPage />,
    children: [
      {
        path: 'allowed_pages',
        element: <AllowedPages />,
      },
    ],
  },
]);


const root = ReactDOM.createRoot(
  document.getElementById('root') as HTMLElement
);
root.render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>
);
