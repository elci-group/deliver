import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import fr from "./locales/fr.json";

i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  resources: { en: { translation: en }, fr: { translation: fr } },
});

export function App({ name }) {
  const greeting = t("welcome", { name });
  const settings = t("menu.settings");
  const logout = t("menu.logout");
  const help = t("menu.help");
  return [greeting, settings, logout, help].join(" ");
}
