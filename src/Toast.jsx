import { useEffect, useState } from "react";

export default function Toast({ message, type = "info" }) {
  const [visible, setVisible] = useState(false);
  const [displayMessage, setDisplayMessage] = useState("");
  const [displayType, setDisplayType] = useState("info");

  useEffect(() => {
    if (message) {
      setDisplayMessage(message);
      setDisplayType(type);
      setVisible(true);
    } else {
      setVisible(false);
    }
  }, [message, type]);

  return (
    <div className={`toast${visible ? " show" : ""}${displayType === "error" || displayType === "warn" ? ` ${displayType}` : ""}`}>
      {displayMessage}
    </div>
  );
}
