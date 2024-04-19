//  Copyright 2024 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause


import "./App.css";
import { Outlet, Route, Routes } from "react-router-dom";
import Main from "./routes/Main";
import Log from "./routes/Log";


function App() {
  return (
    <div>
      <Routes>
        <Route path="/" element={<Outlet />}>
          <Route index element={<Main />} />
          <Route path="log/:name/:format" element={<Log />} />
          <Route path="*" element={<div>Page not found</div>} />
        </Route>
      </Routes>
    </div>
  );
}

export default App;
