import type { Metadata } from "next";
import CompanyDossierClient from "./company-dossier-client";

export function generateMetadata({ params }: { params: { id: string } }): Metadata {
  return {
    title: `Company ${params.id} — ApexIntel`,
    description: `Company dossier for ${params.id} — profile, capabilities, sites, and graph relationships.`,
    openGraph: {
      title: `Company ${params.id} — ApexIntel`,
      description: `Company dossier for ${params.id}`,
      type: "website",
    },
  };
}

export default function CompanyDossierPage({ params }: { params: { id: string } }) {
  return <CompanyDossierClient id={params.id} />;
}
