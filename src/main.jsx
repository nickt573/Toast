import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { initMediaPaths } from "./mediaPaths";

// mediaSrc is called synchronously during render, so the app data dir must be
// cached before anything mounts
initMediaPaths().then(() => {
  ReactDOM.createRoot(document.getElementById("root")).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
});
