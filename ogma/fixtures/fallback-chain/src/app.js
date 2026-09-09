import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import fr from "./locales/fr.json";
import frCA from "./locales/fr-CA.json";

i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  resources: {
    en: { translation: en },
    fr: { translation: fr },
    "fr-CA": { translation: frCA },
  },
});

export function App() {
  const title = t("title");
  const file = t("menu.file");
  const edit = t("menu.edit");
  const view = t("menu.view");
  return [title, file, edit, view].join(" ");
}
