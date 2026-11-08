# Build the checked-in composition from the existing reusable content definitions.
$ErrorActionPreference='Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$destination=Join-Path $PWD 'content/m4'
function Write-M4Package($Relative,$Manifest,$World) {
    $dir=Join-Path $destination $Relative
    New-Item -ItemType Directory -Force $dir|Out-Null
    [IO.File]::WriteAllText((Join-Path $dir 'manifest.json'),($Manifest|ConvertTo-Json -Depth 70))
    [IO.File]::WriteAllText((Join-Path $dir 'world.json'),($World|ConvertTo-Json -Depth 70))
}
function New-M4World($Id,$Capabilities) {
    @{schema_version=2;id=$Id;title=$Id;capabilities=$Capabilities;entities=@();prefabs=@{};resources=@{};systems=@()}
}
function Literal-M4($Value) { @{kind='literal';value=$Value} }
function Variable-M4($Id) { @{kind='variable';id=$Id} }
function Eq-M4($Left,$Right) { @{kind='eq';left=$Left;right=$Right} }
function Set-M4($Id,$Value) { @{kind='set';variable=$Id;value=(Literal-M4 $Value)} }
function Event-M4($Fields,$Publisher) { @{version=1;publisher=$Publisher;fields=$Fields} }
function Import-M4($Package,$Fields) { @{package=$Package;version=1;consumer='behavior.execute';fields=$Fields} }

$action=Get-Content 'content/action/world.json' -Raw|ConvertFrom-Json -AsHashtable
$hero=New-M4World 'm4-hero' @{core=1;render=1;action=1;animation=1;audio=1}
$hero.systems=@('animation.evaluate','audio.events','action.simulate')
$hero.prefabs=$action.prefabs
$hero.entities=@($action.entities|Where-Object{$_.id -in @('hero','enemy/guard','camera')})
($hero.entities|Where-Object{$_.id -eq 'hero'}).components['core.transform'].position=@(4.5,0,3.5)
($hero.entities|Where-Object{$_.id -eq 'hero'}).components['core.health']=@{value=300;max=300}
($hero.entities|Where-Object{$_.id -eq 'enemy/guard'}).components['core.transform'].position=@(4.5,0,1.8)
foreach($id in @('action.session','action.abilities','animation.library','audio.library','input.bindings')) {$hero.resources[$id]=$action.resources[$id]}
$hero.resources['action.session'].bounds=@{min=@(-12,-10);max=@(12,10)}
$hero.resources['action.session'].obstacles=@()
$hero.resources['action.session'].enemy_notice_range=1.1
$heroManifest=@{id='m4.hero';version=1;content_version=1;world='world.json';capabilities=$hero.capabilities;exports=@{
    events=@{
        'action.defeated'=(Event-M4 @{entity='text';faction='int'} 'action.simulate')
        'action.victory'=(Event-M4 @{} 'action.simulate')
    }
    resources=@{'action.abilities'=@{version=1;owner='action.simulate'}}
}}
Write-M4Package 'packs/hero' $heroManifest $hero
Copy-Item -LiteralPath 'content/action/fighter.glb' -Destination "$destination/packs/hero/fighter.glb"
Copy-Item -LiteralPath 'content/action/fighter-alt.glb' -Destination "$destination/packs/hero/fighter-alt.glb"
$heroManifest.assets=@{}
foreach($file in @('fighter.glb','fighter-alt.glb')) {
    $heroManifest.assets[$file]=(Get-FileHash -LiteralPath "$destination/packs/hero/$file" -Algorithm SHA256).Hash.ToLowerInvariant()
}
Write-M4Package 'packs/hero' $heroManifest $hero

$rts=Get-Content 'content/rts-bench/world.json' -Raw|ConvertFrom-Json -AsHashtable
$squad=New-M4World 'm4-squad' @{core=1;render=1;rts=1}
$squad.systems=@('rts.simulate')
$squad.prefabs=@{'squad.worker'=$rts.prefabs.worker;'squad.soldier'=$rts.prefabs.soldier}
for($i=0;$i -lt 6;$i++){
    $role=if($i -lt 4){'worker'}else{'soldier'}
    $squad.entities+=@{id="squad/$role/$i";label="Squad $role $i";prefab="squad.$role";components=@{'core.transform'=@{position=@((-7.5+($i%2)),0,(0.5+[math]::Floor($i/2)))}}}
}
$base=($rts.entities|Where-Object{$_.id -eq 'base/0'})
$base.id='squad/base'
$base.components['core.transform'].position=@(-9.5,0,0.5)
$squad.entities+=,$base
$squad.entities+=@{id='squad/ore';label='Signal crystal';components=@{
    'core.transform'=@{position=@(-4.5,0,-2.5)};'rts.resource'=@{amount=160}
    'render.mesh'=@{parts=@(@{color=@(0.18,0.78,0.79);emission=0.8;transform=@{position=@(0,0.45,0);scale=@(0.8,0.9,0.8)}})}
}}
$squad.entities+=@{id='squad/camera';label='Tactical camera';components=@{
    'core.transform'=@{position=@(0,17,13);rotation=@(-0.43837115,0,0,0.89879405)}
    'render.camera'=@{projection=@{kind='orthographic';height=25};near=0.1;far=120;viewport=@(0,0,1,1);active=$true}
}}
$session=$rts.resources['rts.session']
$session.grid=@{width=24;height=20;cell_size=1;origin=@(-12,-10);blocked=@()}
$session.camera='squad/camera'
$session.ui_bindings=@{}
$session.max_units=32
$session.initial_balance=@(160,0)
$session.auto_combat=$false
$session.recipes.worker.prefab='squad.worker'
$session.recipes.soldier.prefab='squad.soldier'
$squad.resources['rts.session']=$session
$squad.resources['input.bindings']=@{axes=@{
    'camera.x'=@{positive='KeyD';negative='KeyA'}
    'camera.z'=@{positive='KeyS';negative='KeyW'}
    'camera.zoom'=@{positive='KeyE';negative='KeyQ'}
}}
$squadManifest=@{id='m4.squad';version=1;content_version=1;world='world.json';capabilities=$squad.capabilities;exports=@{
    events=@{'rts.delivered'=(Event-M4 @{amount='int';faction='int'} 'rts.simulate');'rts.produced'=(Event-M4 @{entity='text'} 'rts.simulate')}
    resources=@{'rts.session'=@{version=1;owner='rts.simulate'}}
}}
Write-M4Package 'packs/squad' $squadManifest $squad

$quest=New-M4World 'm4-quest' @{core=1;behavior=1;narrative=1;ui=1}
$quest.systems=@('behavior.execute')
$check=@{kind='emit';event='quest.check'}
$quest.resources['behavior.program']=@{
    variables=@{'quest.accepted'=$false;'quest.supplies'=0;'quest.boss_down'=$false;'quest.completed'=$false;'quest.trained'=0}
    events=@{'quest.check'=@{};'rts.delivered'=@{fields=@{amount='int';faction='int'}};'rts.produced'=@{fields=@{entity='text'}};'action.defeated'=@{fields=@{entity='text';faction='int'}};'action.victory'=@{}}
    rules=@(
        @{id='quest.delivery';event='rts.delivered';when=(Eq-M4 @{kind='event';field='faction'} (Literal-M4 0));actions=@(@{kind='add';variable='quest.supplies';value=@{kind='event';field='amount'}},$check)},
        @{id='quest.production';event='rts.produced';actions=@(@{kind='add';variable='quest.trained';value=(Literal-M4 1)})},
        @{id='quest.boss';event='action.victory';actions=@((Set-M4 'quest.boss_down' $true),$check)},
        @{id='quest.hero_fell';event='action.defeated';when=(Eq-M4 @{kind='event';field='entity'} (Literal-M4 'hero'));actions=@(@{kind='go_to';node='quest.failed'})},
        @{id='quest.complete';event='quest.check';when=@{kind='all';conditions=@(
            (Eq-M4 (Variable-M4 'quest.accepted') (Literal-M4 $true)),
            @{kind='ge';left=(Variable-M4 'quest.supplies');right=(Literal-M4 8)},
            (Eq-M4 (Variable-M4 'quest.boss_down') (Literal-M4 $true)),
            (Eq-M4 (Variable-M4 'quest.completed') (Literal-M4 $false))
        )};actions=@((Set-M4 'quest.completed' $true),@{kind='go_to';node='quest.reward'})}
    )
}
$quest.resources['narrative.graph']=@{start='quest.arrival';nodes=@(
    @{id='quest.arrival';speaker='commander';text='arrival';choices=@(
        @{id='accept';text='accept';actions=@((Set-M4 'quest.accepted' $true),$check);next='quest.active'},
        @{id='decline';text='decline';next='quest.declined'}
    )},
    @{id='quest.active';speaker='commander';text='active';choices=@(
        @{id='assault';text='assault';when=@{kind='ge';left=(Variable-M4 'quest.supplies');right=(Literal-M4 8)};next='quest.assault'}
    )},
    @{id='quest.assault';speaker='commander';text='fight';choices=@()},
    @{id='quest.reward';speaker='commander';text='reward';choices=@();ending='joined_victory'},
    @{id='quest.declined';speaker='commander';text='declined';choices=@();ending='declined'},
    @{id='quest.failed';speaker='commander';text='failed';choices=@();ending='hero_fell'}
)}
$quest.resources['ui.document']=@{locale='zh-TW';flow=$true;session_controls=$false;catalogs=@{
    'zh-TW'=@{commander='艾拉／聯隊指揮官';arrival='交付 8 份晶礦，再讓英雄與小隊合力擊敗守衛。G 切換操控；可按「小隊支援守衛戰」。';accept='接受聯隊任務';decline='這次先不參與';active='小隊視角可框選、右鍵採集，或按「派出礦工」。交付 8 份後準備進攻。';assault='補給已就緒，前往英雄視角戰鬥';fight='G 切英雄，F 鎖定、左鍵三段連擊、Space 翻滾。';reward='補給與戰鬥都已完成。聯隊守住了信標！';declined='聯隊仍在此等待你的下一次決定。';failed='英雄倒下了，重開關卡再次挑戰。';support='小隊支援守衛戰';gather='派出礦工採集';train='生產一名士兵';hero='英雄視角';squad='小隊視角'}
    en=@{commander='Aela / Commander';arrival='Deliver 8 crystals with the squad, then defeat the courtyard guard with your hero. G changes control.';accept='Accept joint operation';decline='Not this time';active='Box-select workers and right-click crystals, or dispatch them below. Deliver 8 supplies.';assault='Supplies ready. Begin the assault';fight='G hero control. F lock, left click combo, Space roll.';reward='Supplies delivered. Guard defeated. The beacon is ours!';declined='The operation awaits another day.';failed='The hero fell. Restart the operation.';support='Squad focus fire';gather='Dispatch miners';train='Train soldier';hero='Hero view';squad='Squad view'}
};widgets=@(
    @{kind='text';id='quest.line';binding=@{kind='line'}},
    @{kind='progress';id='quest.supply_bar';variable='quest.supplies';max=8},
    @{kind='choices';id='quest.choices'},
    @{kind='button';id='quest.gather';text='gather';command=@{kind='call';capability='rts';action='gather';args=@{units=@('squad/worker/0','squad/worker/1','squad/worker/2','squad/worker/3');target='squad/ore'}}},
    @{kind='button';id='quest.train';text='train';command=@{kind='call';capability='rts';action='train';args=@{building='squad/base';recipe='soldier';count=1}}},
    @{kind='button';id='quest.support';text='support';command=@{kind='call';capability='rts';action='attack';args=@{units=@('squad/soldier/4','squad/soldier/5');target='enemy/guard'}}}
)}
$questManifest=@{id='m4.quest';version=1;content_version=1;world='world.json';capabilities=$quest.capabilities;dependencies=@{
    'm4.hero'=@{path='../hero';content_version=1};'m4.squad'=@{path='../squad';content_version=1}
};imports=@{
    events=@{
        'rts.delivered'=(Import-M4 'm4.squad' @{amount='int';faction='int'})
        'rts.produced'=(Import-M4 'm4.squad' @{entity='text'})
        'action.defeated'=(Import-M4 'm4.hero' @{entity='text';faction='int'})
        'action.victory'=(Import-M4 'm4.hero' @{})
    }
    resources=@{
        'action.abilities'=@{package='m4.hero';version=1;consumer='behavior.execute'}
        'rts.session'=@{package='m4.squad';version=1;consumer='behavior.execute'}
    }
}}
Write-M4Package 'packs/quest' $questManifest $quest

$root=New-M4World 'beacon-alliance' @{core=1;render=1;input=1;combat=1}
$root.systems=@('combat.resolve')
$root.resources['combat.session']=@{bounds=@{min=@(-12,-10);max=@(12,10)};obstacles=@()}
$root.title='Beacon Alliance / RTS + Action + ADV'
$root.resources['render.settings']=@{accent=@(0.20,0.83,0.73);shadow_extent=27}
$root.resources['input.bindings']=@{buttons=@{'context.next'='KeyG'}}
$root.resources['input.contexts']=@{initial='squad';contexts=@{
    hero=@{label='Hero / 英雄';systems=@('action.simulate');cameras=@('camera');capture_pointer=$true}
    squad=@{label='Squad / 小隊';systems=@('rts.simulate');cameras=@('squad/camera');capture_pointer=$false}
}}
$root.entities=@(
    @{id='alliance/floor';label='Shared operation ground';components=@{'core.transform'=@{position=@(0,-0.18,0);scale=@(24,0.32,20)};'render.mesh'=@{parts=@(@{color=@(0.11,0.18,0.21);grid=$true})}}},
    @{id='alliance/beacon';label='Alliance Beacon';components=@{'core.transform'=@{position=@(0,0,-4)};'render.mesh'=@{parts=@(
        @{color=@(0.22,0.30,0.34);transform=@{position=@(0,0.4,0);scale=@(1.4,0.8,1.4)}},
        @{color=@(0.20,0.88,0.76);emission=1.5;transform=@{position=@(0,1.4,0);scale=@(0.32,1.3,0.32)}}
    )}}},
    @{id='alliance/court';label='Hero courtyard';components=@{'core.transform'=@{position=@(4.5,0,-1.5)};'render.mesh'=@{parts=@(
        @{color=@(0.24,0.29,0.32);transform=@{position=@(-2,1.1,0);scale=@(0.3,2.2,0.3)}},
        @{color=@(0.24,0.29,0.32);transform=@{position=@(2,1.1,0);scale=@(0.3,2.2,0.3)}},
        @{color=@(0.95,0.44,0.16);emission=1;transform=@{position=@(0,2.2,0);scale=@(4.3,0.15,0.3)}}
    )}}}
)
$manifest=@{id='m4.alliance';version=1;content_version=2;world='world.json';capabilities=$root.capabilities;dependencies=@{'m4.quest'=@{path='packs/quest';content_version=1}};exports=@{events=@{
    'combat.hit'=(Event-M4 @{source='text';target='text';amount='number'} 'combat.resolve')
    'combat.defeated'=(Event-M4 @{entity='text';source='text';faction='int'} 'combat.resolve')
}}}
Write-M4Package '.' $manifest $root
