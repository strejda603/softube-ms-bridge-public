#!/bin/bash
# sleep 10
# Step 1: Enable iDAM on iPad using Audio MIDI Setup on macOS
osascript <<'EOF'
-- Kontrola, zda aplikace běží. Pokud ne, spustí se a počká na její načtení.
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

tell application "System Events"
	tell process "Audio MIDI Setup"
		set oknoAudio to missing value
		set seznamOken to every window
		set nazevZarizeni to "Daniel Pitra - iPad"

		repeat with w in seznamOken
			if name of w contains "Reproduktory" or name of w contains "Mikrofon" or name of w contains "zařízení" or name of w contains "BlackHole" then
				set oknoAudio to w
				exit repeat
			end if
		end repeat

		if oknoAudio is missing value then
			log "Okno Audio se nepodařilo nalézt."
			return
		end if

		tell oknoAudio
			tell splitter group 1 to tell scroll area 1 to tell outline 1
				repeat with i from 1 to count rows
					try
						set rowName to name of static text 1 of UI element 1 of row i
						if rowName is equal to nazevZarizeni then
							select row i
							delay 0.2
							click button "Zapnout" of UI element nazevZarizeni of row i

							-- Oprava: Skrytí aplikace musí proběhnout PŘED opuštěním cyklu (exit repeat)
							set visible of process "Audio MIDI Setup" to false
							exit repeat
						end if
					on error
						-- log "Chyba"
					end try
				end repeat
			end tell
		end tell
	end tell
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
open -a "Softube Console 1 MS Bridge"
# Step 5: Open Ableton Project
open -a "Ableton Live 12 Suite" "/Users/danielpitra/Documents/Ableton Projects/STO/STO Project/STO.als"
