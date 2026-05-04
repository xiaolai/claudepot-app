import { Cpu } from "lucide-react";
import { readAudienceMd } from "@/lib/editorial-spec";
import { renderEditorialDoc } from "@/lib/markdown";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";

export const dynamic = "force-static";

export default function VoicePage() {
  const html = renderEditorialDoc(readAudienceMd());
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="voice" />
      <div className="proto-page-aside-content">
        <header className="proto-section">
          <div className="office-eyebrow">
            <Cpu size={14} aria-hidden /> voice + audience
          </div>
        </header>
        <article
          className="office-doc"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </div>
    </div>
  );
}
