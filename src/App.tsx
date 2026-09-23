import { Route, Routes } from "react-router-dom";

import { AppShell } from "@/components/AppShell";
import { Home } from "@/screens/Home";
import { Settings } from "@/screens/Settings";
import { Brief } from "@/screens/Brief";
import { Run } from "@/screens/Run";
import { Report } from "@/screens/Report";

function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<Home />} />
        <Route path="settings" element={<Settings />} />
        <Route path="session/:id/brief" element={<Brief />} />
        <Route path="session/:id/run" element={<Run />} />
        <Route path="session/:id/report" element={<Report />} />
      </Route>
    </Routes>
  );
}

export default App;
