import { Table, TableBody, TableCell, TableHead, TableRow, TableContainer } from '@mui/material';
import React, { useEffect, useState } from 'react'

export default function AllowedPages() {
  const [allowedPages, setAllowedPages] = useState<string[]>([]);
  const [currentUrl, setCurrentUrl] = useState("")
  const [currentTabId, setCurrentTabId] = useState(0)

  const loadAllowedWebPages = () => {
    chrome.runtime.sendMessage({ event: "getAllowedPages" }).then((response) => {
      setAllowedPages(response);
    })
  }


  useEffect(() => {
    loadAllowedWebPages();
    chrome.tabs.query({ active: true, lastFocusedWindow: true }, (tabs) => {
      let [tab] = tabs;
      if (tab?.url) {
        let domain = (new URL(tab.url));
        console.log(domain.hostname)
        setCurrentUrl(domain.hostname);
      }
      if (tab?.id) {
        setCurrentTabId(tab.id);
      }
    });
  }, []);

  const toggleCurrentWebpage = () => {
    if (allowedPages.includes(currentUrl)) {
      removeUrl(currentUrl);
    } else {
      chrome.runtime.sendMessage({ event: "addAllowedPage", url: currentUrl }).then(setAllowedPages);
      chrome.tabs.sendMessage(currentTabId, { event: "inject" });
    }
  }

  const removeUrl = (url: String) => {
    chrome.runtime.sendMessage({ event: "removeAllowedPage", url }).then(setAllowedPages);
  }

  return (
    <>
      <TableContainer>
        <Table>
          <TableHead>
            <TableRow>
              <TableCell>
                URL
              </TableCell>
              <TableCell>
                Remove?
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {allowedPages.map((url) => <TableRow><TableCell>{url}</TableCell><TableCell onClick={() => removeUrl(url)}>remove</TableCell></TableRow>)}
          </TableBody>
        </Table>
      </TableContainer>
      <hr />
      <div onClick={toggleCurrentWebpage}>{allowedPages.includes(currentUrl) ? "Remove" : "Add"} current web page</div>
    </>
  )
}
