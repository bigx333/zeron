-- Settings → Orchestration: what the engines do with orchestrators' work.
-- For now, crew threads (chats an orchestrator started) left idle: archived
-- on their own, and after how long. The profile's, synced and read by every
-- engine (account/orchestration `settings`), not this device's alone.

local ds = require("@ds")
local search = require("@app/screens/settings/search")
local orchestration = require("@account/orchestration")

local M = {}

M.ID = "orchestration"
M.TITLE = "Orchestration"

-- Found by the settings search (indexed as it loads, and again as
-- Orchestrators comes back on: screens/settings/init).
function M.index()
	search.index(M.ID, ui.t("settings.orchestration.title"), ui.t("settings.orchestration.terms"), {
		{ key = "auto_archive", title = ui.t("settings.orchestration.auto_archive.label"), section = ui.t("settings.orchestration.crew_section"), terms = ui.t("settings.orchestration.auto_archive.terms") },
		{ key = "idle_hours", title = ui.t("settings.orchestration.idle_hours.label"), section = ui.t("settings.orchestration.crew_section"), terms = ui.t("settings.orchestration.idle_hours.terms") },
		{ key = "orchestrator_prompt", title = ui.t("settings.orchestration.orchestrator_prompt.label"), section = ui.t("settings.orchestration.prompts_section"), terms = ui.t("settings.orchestration.orchestrator_prompt.terms") },
		{ key = "crew_prompt", title = ui.t("settings.orchestration.crew_prompt.label"), section = ui.t("settings.orchestration.prompts_section"), terms = ui.t("settings.orchestration.crew_prompt.terms") },
		{ key = "brain_dump_prompt", title = ui.t("settings.orchestration.brain_dump_prompt.label"), section = ui.t("settings.orchestration.prompts_section"), terms = ui.t("settings.orchestration.brain_dump_prompt.terms") },
	})
end
M.index()

local function on(): boolean
	return orchestration.auto_archive:get() == true
end

-- How long idle first, as the menu says it (a day and more in days).
local function idle_options(): { { value: number, label: string } }
	local out = {}
	for _, h in orchestration.IDLE_HOURS do
		table.insert(out, {
			value = h,
			label = if h >= 24 and h % 24 == 0
				then ui.t("settings.orchestration.idle_hours.days", { n = h // 24 }):get()
				else ui.t("settings.orchestration.idle_hours.hours", { n = h }):get(),
		})
	end
	-- (A window set elsewhere, not one of these: listed too.)
	local now = orchestration.idle_hours:get()
	if type(now) == "number" and table.find(orchestration.IDLE_HOURS, now) == nil then
		table.insert(out, { value = now, label = ui.t("settings.orchestration.idle_hours.hours", { n = now }):get() })
	end
	return out
end

-- The three role prompts, editable: an orchestrator's, its crew's and the
-- Brain dump's. Each row's draft is the engine's (synced) value until the
-- person types; Save writes it through `updateSettings`, Reset writes `""`
-- which the engine reads as the built-in.
local PROMPT_KEYS = { "orchestrator_prompt", "crew_prompt", "brain_dump_prompt" }

local function prompt_row(key: string): Node
	local draft = ui.state(`orch_prompt_draft_{key}`, "")
	-- What the engine last said (or this row saved): the draft apart from
	-- it is the person's own typing, and the engine's next word doesn't
	-- overwrite it.
	local synced = ui.state(`orch_prompt_synced_{key}`, nil :: string?)
	local saving = ui.state(`orch_prompt_saving_{key}`, false)
	local save_error = ui.state(`orch_prompt_error_{key}`, "")
	-- What Save last sent, while the engine's answer is still owed: a draft
	-- that is only it is the person's own typing still, so the engine's
	-- normalized word (trimmed, or the default a blank resets to) replaces
	-- it rather than reading as someone else's edit.
	local pending_sent: string? = nil

	local function trim(s: string): string
		return string.match(s, "^%s*(.-)%s*$") or ""
	end

	ui.effect(function()
		local v = ((orchestration.settings:get() :: any)[key]) :: string?
		if v ~= nil and v ~= synced:peek() then
			local untouched = synced:peek() == nil
				or draft:peek() == synced:peek()
				or (pending_sent ~= nil and draft:peek() == pending_sent)
			synced:set(v)
			if untouched then
				draft:set(v)
				pending_sent = nil
			end
		end
	end)

	local function saved(): string
		return ((orchestration.settings:get() :: any)[key]) or ""
	end
	local function default_text(): string
		return ((orchestration.settings:get() :: any)[`{key}_default`]) or ""
	end
	local function custom(): boolean
		return ((orchestration.settings:get() :: any)[`{key}_custom`]) == true
	end

	local function save()
		pending_sent = draft:peek()
		saving:set(true)
		save_error:set("")
		orchestration.set_prompt(key, pending_sent :: string):after(function()
			saving:set(false)
			-- The engine trims, and reads a blank or the built-in itself as
			-- a reset; when nothing new arrives (a no-op after the trim, no
			-- stream frame), snap to what it now says all the same.
			if draft:peek() == pending_sent then
				local cur = ((orchestration.settings:peek() :: any)[key]) :: string?
				local sent = trim(pending_sent :: string)
				if cur ~= nil and (cur == sent or (sent == "" and cur == default_text())) then
					synced:set(cur)
					draft:set(cur)
					pending_sent = nil
				end
			end
		end, function(err: any)
			saving:set(false)
			pending_sent = nil
			save_error:set(orchestration.reason(err))
		end)
	end

	local function reset()
		save_error:set("")
		if not custom() then
			-- Already the built-in: nothing for the engine to do, the box
			-- alone goes back.
			draft:set(default_text())
			return
		end
		saving:set(true)
		orchestration.reset_prompt(key):after(function()
			saving:set(false)
			draft:set(((orchestration.settings:peek() :: any)[`{key}_default`]) or "")
		end, function(err: any)
			saving:set(false)
			save_error:set(orchestration.reason(err))
		end)
	end


	return ds.Settings.Row {
		label = ui.t(`settings.orchestration.{key}.label`),
		description = ui.t(`settings.orchestration.{key}.description`),
		stacked = true,
		control = ui.column {
			id = `orch-prompt-{key}`,
			w_full = true,
			gap = "$space.2",
			ds.TextArea {
				value = draft,
				name = ui.t(`settings.orchestration.{key}.label`),
				rows = 6,
				max_rows = 16,
				w_full = true,
				on_submit = save,
			},
			ui.row {
				w_full = true,
				justify = "between",
				items = "center",
				gap = "$space.2",
				ui.text { function()
					local err = save_error:get()
					if err ~= "" then
						return err
					end
					return if custom() then ui.t("settings.orchestration.prompts.custom"):get() else ui.t("settings.orchestration.prompts.default"):get()
				end, color = function()
					return if save_error:get() ~= "" then "$zeron.tone.danger.ink" else "$text.tertiary"
				end, text_size = "$font.size.sm" },
				ui.row {
					gap = "$space.2",
					items = "center",
					ds.Button {
						ui.t("settings.orchestration.prompts.reset"),
						tone = "ghost",
						size = "sm",
						disabled = function()
							return not custom() and draft:get() == default_text()
						end,
						on_click = reset,
					},
					ds.Button {
						ui.t("settings.orchestration.prompts.save"),
						tone = "primary",
						size = "sm",
						busy = saving,
						disabled = function()
							return draft:get() == saved()
						end,
						on_click = save,
					},
				},
			},
		},
	}
end

-- The "Role prompts" section: the three rows, each found by the search.
local function prompts_section(): Node
	local rows = {}
	for _, key in PROMPT_KEYS do
		table.insert(rows, search.anchor(key, prompt_row(key)))
	end
	return ds.Settings.Section {
		title = ui.t("settings.orchestration.prompts_section"),
		ds.Settings.Group(rows),
	}
end

function M.view(): Node
	return ds.Settings.Page {
		title = ui.t("settings.orchestration.title"),
		description = ui.t("settings.orchestration.description"),
		ds.Settings.Section {
			title = ui.t("settings.orchestration.crew_section"),
			ds.Settings.Group {
				search.anchor(
					"auto_archive",
					ds.Settings.Row {
						label = ui.t("settings.orchestration.auto_archive.label"),
						description = ui.t("settings.orchestration.auto_archive.description"),
						control = ds.Switch { value = orchestration.auto_archive, label = ui.t("settings.orchestration.auto_archive.label") },
					}
				),
				-- (Holds only while archiving is on: dimmed and out of reach
				-- while it's off.)
				search.anchor(
					"idle_hours",
					ui.div {
						w_full = true,
						opacity = function()
							return if on() then 1 else "$zeron.disabled"
						end,
						disabled = function()
							return not on()
						end,
						ds.Settings.Row {
							label = ui.t("settings.orchestration.idle_hours.label"),
							description = ui.t("settings.orchestration.idle_hours.description"),
							control = ds.Select {
								value = orchestration.idle_hours,
								name = ui.t("settings.orchestration.idle_hours.label"),
								w = 160,
								options = idle_options,
							},
						},
					}
				),
			},
		},
		prompts_section(),
	}
end

return M
