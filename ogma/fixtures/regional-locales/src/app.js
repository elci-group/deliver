import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import enGB from "./locales/en-GB.json";
import enUS from "./locales/en-US.json";
import frFR from "./locales/fr-FR.json";

i18n.use(initReactI18next).init({
  lng: "en-GB",
  fallbackLng: "en-GB",
  resources: {
    "en-GB": { translation: enGB },
    "en-US": { translation: enUS },
    "fr-FR": { translation: frFR },
  },
});

export function App() {
  const colour = t("colour.label");
  const catalogue = t("catalogue.title");
  const save = t("action.save");
  return [colour, catalogue, save].join(" ");
}
