import { IconContext } from "@phosphor-icons/react";
import { SectionRail } from "./components/SectionRail";
import { sections, sectionIds } from "./sections/registry";
import { useSection } from "./hooks/useSection";

function App() {
  const { section, setSection } = useSection(sectionIds[0], sectionIds);
  const ActiveComponent =
    (sections.find((s) => s.id === section) ?? sections[0]).Component;

  return (
    <IconContext.Provider value={{ size: 16, weight: "light" }}>
      <div className="app-layout">
        <div className="titlebar-drag" data-tauri-drag-region />
        <SectionRail sections={sections} active={section} onSelect={setSection} />
        <ActiveComponent />
      </div>
    </IconContext.Provider>
  );
}

export default App;
