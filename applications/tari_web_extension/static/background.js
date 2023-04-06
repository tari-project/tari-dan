var allowedWebPages = []
// Get current saved state when browser starts
chrome.storage.local.get('allowedWebPages', (result) => {
  if (result.allowedWebPages) {
    allowedWebPages = result.allowedWebPages;
    console.log('loaded', allowedWebPages);
  }
});

function addAllowedPage(url) {
  allowedWebPages.push(url);
  console.log('add ', url);
  chrome.storage.local.set({ 'allowedWebPages': allowedWebPages });
  return allowedWebPages;
}

function removeAllowedPage(url) {
  allowedWebPages = allowedWebPages.filter((item) => item !== url);
  console.log('remove ', url);
  chrome.storage.local.set({ 'allowedWebPages': allowedWebPages });
  return allowedWebPages;
}

function isPageAllowed(url) {
  return allowedWebPages.includes(url)
}

chrome.runtime.onMessage.addListener(async (request, sender, sendResponse) => {
  console.log("request", request);
  if (sender.origin.startsWith("chrome-extension")) {
    // These are the calls from popup
    switch (request?.event) {
      case 'getAllowedPages': sendResponse(allowedWebPages); break;
      case 'addAllowedPage': sendResponse(addAllowedPage(request.url)); break;
      case 'removeAllowedPage': sendResponse(removeAllowedPage(request.url)); break;
      default: console.log("Unknown event"); break;
    }
  } else {
    // These are the calls from content script
    switch (request?.event) {
      case 'isPageAllowed': sendResponse(isPageAllowed(request.url)); break;
      default: console.log("Unknown event"); break;
    }
  }
});
