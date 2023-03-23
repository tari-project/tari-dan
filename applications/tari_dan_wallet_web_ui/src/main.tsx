import React from 'react'
import ReactDOM from 'react-dom/client'
import { createBrowserRouter, RouterProvider } from 'react-router-dom';
import App from './App'
import './index.css'
import Accounts from './routes/Accounts/Accounts';

const router = createBrowserRouter([
  {
    path: '*',
    element: <App />,
    errorElement: <div />,
    children: [
      {
        path: 'accounts',
        element: <Accounts />,
      },
    ],
  },
]);

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
)
