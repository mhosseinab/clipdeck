import React from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import SettingsPage from "./Settings";
import "./styles.css";

const isSettings = window.location.hash.startsWith("#/settings");
createRoot(document.getElementById("root")!).render(
  <React.StrictMode>{isSettings ? <SettingsPage /> : <App />}</React.StrictMode>,
);
