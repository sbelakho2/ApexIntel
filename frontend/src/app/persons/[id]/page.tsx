import type { Metadata } from "next";
import PersonDossierClient from "./person-dossier-client";

export function generateMetadata({ params }: { params: { id: string } }): Metadata {
  return {
    title: `POI ${params.id} — ApexIntel`,
    description: `Person-of-interest dossier for ${params.id} — priority vector, engagement guide, and profile.`,
    openGraph: {
      title: `POI ${params.id} — ApexIntel`,
      description: `POI dossier for ${params.id}`,
      type: "website",
    },
  };
}

export default function PersonDossierPage({ params }: { params: { id: string } }) {
  return <PersonDossierClient id={params.id} />;
}
