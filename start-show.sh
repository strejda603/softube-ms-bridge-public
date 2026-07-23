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

# Step 6: Force Ableton's Audio Output Device to "SPD-SX PRO".
# Live remembers this as a global app preference, not per-project, so something else
# (a different show, a driver reset, macOS switching the system default device) can leave
# it pointed elsewhere. Re-assert it every run instead of trusting it stuck from last time.
osascript <<'EOF'
tell application "System Events"
	-- Wait for Live to actually launch (a big project can take a while to load).
	set t to 0
	repeat until (exists process "Live") or t > 300
		delay 0.2
		set t to t + 1
	end repeat
	if not (exists process "Live") then
		log "Ableton Live did not launch in time; skipping Audio Output Device check."
		return
	end if
	-- Extra grace period for the project itself to finish loading before we touch menus.
	delay 5

	tell application "Ableton Live 12 Suite" to activate
	delay 0.5

	tell process "Live"
		set frontmost to true

		-- Open Preferences via the app menu (menu item name may or may not include "…").
		try
			set liveMenu to menu bar item "Live" of menu bar 1
			set prefsItem to missing value
			repeat with mi in menu items of menu 1 of liveMenu
				if name of mi contains "Preferences" then
					set prefsItem to mi
					exit repeat
				end if
			end repeat
			if prefsItem is missing value then error "Preferences menu item not found"
			click prefsItem
		on error errMsg
			log "Could not open Ableton Preferences via menu: " & errMsg
			return
		end try
		delay 1

		if not (exists window 1) then
			log "Ableton Preferences window did not appear."
			return
		end if
		set prefsWindow to window 1

		-- Preferences has a tab list on the left; click whichever one is/contains "Audio".
		try
			set audioTabFound to false
			repeat with uiEl in UI elements of prefsWindow
				try
					if (role of uiEl is "AXButton" or role of uiEl is "AXRadioButton") and (name of uiEl contains "Audio") then
						click uiEl
						set audioTabFound to true
						exit repeat
					end if
				end try
			end repeat
			if not audioTabFound then log "Could not find an 'Audio' tab; assuming Audio page is already showing."
		on error errMsg
			log "Error while looking for the Audio tab: " & errMsg
		end try
		delay 0.5

		-- Find a pop up button that looks like the output-device selector and pick "SPD-SX PRO".
		try
			set outputPopup to missing value
			repeat with pb in pop up buttons of prefsWindow
				try
					if (title of pb contains "Output") or (value of pb is not missing value) then
						-- Prefer one whose menu actually lists our target device.
						click pb
						delay 0.3
						if exists (menu item "SPD-SX PRO" of menu 1 of pb) then
							set outputPopup to pb
							exit repeat
						else
							-- Not the right popup; close its menu and keep looking.
							key code 53 -- Escape
							delay 0.2
						end if
					end if
				end try
			end repeat

			if outputPopup is not missing value then
				click menu item "SPD-SX PRO" of menu 1 of outputPopup
				delay 0.3
				log "Ableton Audio Output Device set to SPD-SX PRO."
			else
				log "Could not find an Output Device popup listing 'SPD-SX PRO' — check it manually."
			end if
		on error errMsg
			log "Failed to set Audio Output Device to SPD-SX PRO: " & errMsg
		end try

		-- Close Preferences.
		try
			click button 1 of prefsWindow
		on error
			try
				keystroke "w" using command down
			end try
		end try
	end tell
end tell
EOF