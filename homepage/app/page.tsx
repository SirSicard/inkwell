import DownloadSection from "@/components/DownloadSection";
import FeaturesSection from "@/components/FeaturesSection";
import Hero from "@/components/Hero";
import HowItWorks from "@/components/HowItWorks";
import ModelsSection from "@/components/ModelsSection";
import PrivacySection from "@/components/PrivacySection";
import SiteFooter from "@/components/SiteFooter";
import SiteHeader from "@/components/SiteHeader";
import SupportSection from "@/components/SupportSection";

export default function Home() {
  return (
    <>
      <a className="skip-link" href="#main">
        Skip to content
      </a>
      <span id="top" />
      <SiteHeader />
      <main id="main">
        <Hero />
        <HowItWorks />
        <FeaturesSection />
        <ModelsSection />
        <PrivacySection />
        <DownloadSection />
        <SupportSection />
      </main>
      <SiteFooter />
    </>
  );
}
