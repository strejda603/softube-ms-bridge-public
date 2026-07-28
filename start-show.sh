#!/bin/bash
# Step 1: Enable iDAM on iPad and connect MIDI Maestro (Bluetooth) using Audio MIDI Setup on macOS
osascript <<'EOF'
-- 1. Kontrola, zda aplikace běží. Pokud ne, spustí se a počká na její načtení.
tell application "System Events"
	if not (exists process "Audio MIDI Setup") then
		tell application "Audio MIDI Setup" to activate
		-- Čeká, dokud se proces neobjeví v systému (maximálně 5 sekund)
		set t to 0
		repeat until (exists process "Audio MIDI Setup") or t > 50
			delay 0.1
			set t to t + 1
		end repeat
		delay 0.5
	else
		tell application "Audio MIDI Setup" to activate
	end if
end tell

-- 2. INTERAKCE V OKNĚ AUDIO MIDI SETUP
tell application "System Events"
	tell process "Audio MIDI Setup"
		set oknoAudio to missing value
		set oknoBT to window "Konfigurace Bluetooth"
		set seznamOken to every window

		-- 2.1 Najdeme okno, které obsahuje "Reproduktory", "Mikrofon", "zařízení" nebo "BlackHole"
		repeat with w in seznamOken
			if name of w contains "Reproduktory" or name of w contains "Mikrofon" or name of w contains "zařízení" or name of w contains "BlackHole" then
				set oknoAudio to w
				exit repeat
			end if
		end repeat

		-- 2.2 Interakce s nalezeným oknem
		tell oknoAudio
			tell splitter group 1 to tell scroll area 1 to tell outline 1
				set celkemRadku to count rows
				set nalezeno to false
				repeat with i from 1 to count rows
					try
						set rowName to name of static text 1 of UI element 1 of row i
						if rowName contains "iPad" then
							-- select row i
							-- delay 0.2
							click button "Zapnout" of UI element 1 of row i
							set nalezeno to true
							exit repeat
						end if
					on error chybovaHlaska
						-- log "Chyba řádku: " & chybovaHlaska
					end try
				end repeat
			end tell
		end tell

		-- 2.3 Interakce s oknem "Konfigurace Bluetooth" pro připojení zařízení "MIDI Maestro"
		tell oknoBT
			tell scroll area 1 to tell table 1
				set nalezeno to false
				set celkemRadku to count rows

				repeat with i from 1 to celkemRadku
					try
						set aktualniRadek to row i
						set textRadku to ""

						-- Prohledáme buňky řádku pro ověření názvu
						tell aktualniRadek
							set vsechnyBunky to every UI element
							repeat with jednaBunka in vsechnyBunky
								try
									set textRadku to textRadku & " " & (name of jednaBunka as string)
								end try
							end repeat
						end tell

						if textRadku contains "MIDI Maestro" then
							log "Zařízení nalezeno na řádku " & i & ". Pokus o stisknutí tlačítka uvnitř buněk..."

							-- OPRAVA: Projdeme elementy (buňky) řádku a klikneme na první tlačítko, které v nich najdeme
							tell aktualniRadek
								repeat with jednaBunka in vsechnyBunky
									-- Pokud buňka obsahuje nějaké tlačítko, klikneme na něj (nezávisle na jeho textovém názvu)
									if (count buttons of jednaBunka) > 0 then
										click button "Připojit" of jednaBunka
										log "Úspěch: Tlačítko pro připojení MIDI Maestro bylo stisknuto."
										set nalezeno to true
										exit repeat
									end if
								end repeat
							end tell

							if nalezeno then exit repeat
						end if
					on error chybovaHlaska
						log "Chyba řádku " & i & ": " & chybovaHlaska
					end try
				end repeat

				if not nalezeno then
					log "Zařízení 'MIDI Maestro' nebylo v seznamu nalezeno nebo se nepodařilo kliknout."
				end if
			end tell
		end tell
	end tell

	-- Skryjeme celou aplikaci Audio MIDI Setup na pozadí
	set visible of process "Audio MIDI Setup" to false
end tell
EOF
sleep 5
# Step 2: Open the Bome MIDI Translator Pro project file
open -a "Bome MIDI Translator Pro" "/Users/danielpitra/Documents/Bome MIDI Translator/Presets/STO.bmtp"
# Step 3: Open Mixing Station (need to adjust appSeries and IP address)
open -a "Mixing Station" --args "-appSeries=X32/M32" -ip=192.168.1.1 -mixTarget=-1
#Step 3.5: Check if "Softube On-Screen Display" app is running, if not, start it
pgrep -f "Softube On-Screen Display" > /dev/null || open -a "Softube On-Screen Display"
# Step 4: Start Softube Console 1 MS Bridge
open -a "Softube Console 1 MS Bridge" --args --preset "STO"
# Step 5: Open Ableton Project
open -a "Ableton Live 12 Suite" "/Users/danielpitra/Documents/Ableton Projects/STO/STO Project/STO.als"
