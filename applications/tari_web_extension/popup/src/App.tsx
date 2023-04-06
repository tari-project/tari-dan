import { Routes, Route, Link, useLocation,redirect ,useNavigate  } from 'react-router-dom';
import AllowedPages from './routes/AllowedPages/AllowedPages';
import './App.css'
import ErrorPage from './routes/ErrorPage';

export default function App() {
  // const rootDirectory = chrome.runtime.getURL('/');
  const location = useLocation();
  console.log(location.pathname); // This will log '/'
  // const location = useLocation();
  // console.log(location.pathname);
    return (
    <div>
      <Routes>
      <Route path="/popup/index.html" element={<AllowedPages/>}/>
      </Routes>
    </div>
  );
}
