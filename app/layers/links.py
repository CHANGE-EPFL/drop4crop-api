from sqlmodel import Field, SQLModel, Relationship
from uuid import UUID
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from app.layers.models import Layer
    from app.countries.models import Country


class LayerCountryLink(SQLModel, table=True):
    country_id: UUID | None = Field(
        default=None,
        foreign_key="country.id",
        primary_key=True,
    )
    layer_id: UUID | None = Field(
        default=None,
        foreign_key="layer.id",
        primary_key=True,
    )

    # Variables
    var_wf: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_wfb: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_wfg: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_vwc: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_vwcb: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_vwcg: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_wdb: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )
    var_wdg: float | None = Field(
        default=None,
        nullable=True,
        index=True,
    )

    # Relationships
    layer: "Layer" = Relationship(
        back_populates="country_values",
        sa_relationship_kwargs={"lazy": "selectin"},
    )
    country: "Country" = Relationship(
        back_populates="layer_values",
        sa_relationship_kwargs={"lazy": "selectin"},
    )
